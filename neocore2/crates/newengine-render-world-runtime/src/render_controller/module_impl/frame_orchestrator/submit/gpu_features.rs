use super::*;

fn record_gpu_indirect_visibility_cull(
    controller: &mut RuntimeRenderController,
    r: &mut dyn RenderApi,
    extraction: &SceneExtractionCtx<'_>,
    frame_camera: newengine_core::render::FrameCameraContext,
    frame_plan: &newengine_render_frame_graph::RenderFramePlan,
) {
    if !frame_plan.contains_phase(newengine_render_frame_graph::StandardRenderPhase::VisibilityCull)
        || !controller.gpu_indirect_gbuffer_ready()
    {
        return;
    }

    let Some(stream) = controller.gpu.indirect_stream.current().cloned() else {
        return;
    };
    if !stream.migration_ready() {
        return;
    }

    let result = crate::render_controller::module_impl::frame_submit::record_render_phase(
        r,
        newengine_core::render::RenderGraphPassKind::VisibilityCull,
        |r| {
            for page in &stream.pages {
                let args = newengine_render_api::GpuVisibilityIndirectCompactArgsV2::new(
                    page.candidate_buffer,
                    0,
                    page.source_indirect_buffer,
                    0,
                    page.output_indirect_buffer,
                    0,
                    page.count_buffer,
                    0,
                    page.draw_count,
                    page.draw_count,
                    [
                        extraction.viewport_extent.width.max(1),
                        extraction.viewport_extent.height.max(1),
                    ],
                    frame_camera,
                );
                r.dispatch_visibility_indirect_compact_v2(args)?;
            }
            Ok(())
        },
    );

    if let Err(error) = result {
        // Fail-open contract: output/count were initialized to the full source stream before
        // graph recording. If V2 cull is unavailable for this frame, legacy/GBuffer migration
        // may still draw the full eligible subset without losing geometry.
        newengine_ulog_api::ulog::warn!(
            "render gpu indirect visibility: compact dispatch skipped fail-open frame={} pages={} err='{}'",
            controller.frame.frame_index,
            stream.pages.len(),
            error,
        );
    }
}

pub(super) fn record_runtime_gpu_features(
    controller: &mut RuntimeRenderController,
    r: &mut dyn RenderApi,
    scene: &Scene,
    extraction: &SceneExtractionCtx<'_>,
    view_matrix: newengine_math::Mat4,
    scene_color_format: TextureFormat,
    scope: RenderFrameScope,
    hair_enabled: bool,
    directional_shadow_rendering: bool,
    frame_camera: newengine_core::render::FrameCameraContext,
    frame_plan: &newengine_render_frame_graph::RenderFramePlan,
) {
    record_gpu_indirect_visibility_cull(controller, r, extraction, frame_camera, frame_plan);

    if hair_enabled {
        match controller.gpu.hair.record_frame(
            r,
            scene.world(),
            controller.frame.frame_index,
            scope.dt,
            extraction.viewproj,
            view_matrix,
            extraction.camera_position,
            extraction.camera_forward,
            extraction.shadow_frame,
            extraction.shadow_plan.extent(),
            directional_shadow_rendering,
            scene_color_format,
            scope.vp_w,
            scope.vp_h,
            extraction.lights.dir_dir_intensity,
            extraction.lights.dir_color,
            extraction.lights.ambient,
        ) {
            Ok(report) => {
                if scope.trace_frame && report.active_instances > 0 {
                    newengine_ulog_api::ulog::debug!(
                        "hair gpu: instances={} guide_points={} guide_strands={} render_segments={} shadow_cascades={} shadow_segments={} topology_uploads={}",
                        report.active_instances,
                        report.guide_points,
                        report.guide_strands,
                        report.rendered_segments,
                        report.shadow_cascades,
                        report.shadow_segments,
                        report.topology_uploads,
                    );
                }
            }
            Err(error) if is_transient_shader_pipeline_error(&error) => {
                newengine_ulog_api::ulog::debug!(
                    "hair gpu: shader/pipeline not ready; frame skipped without disabling scene rendering: {}",
                    error
                );
            }
            Err(error) => {
                newengine_ulog_api::ulog::warn!(
                    "hair gpu: frame realization skipped without disabling scene rendering: {}",
                    error
                );
            }
        }
    }
    let vfx_texture_paths = scene
        .world()
        .resource::<newengine_vfx_api::VfxGpuTextureRegistry>()
        .map(|registry| registry.slots().clone())
        .unwrap_or_default();
    let mut vfx_texture_slots = [None; newengine_vfx_api::VFX_GPU_TEXTURE_SLOT_CAPACITY];
    for (index, path) in vfx_texture_paths.iter().enumerate() {
        let Some(path) = path.as_deref() else {
            continue;
        };
        vfx_texture_slots[index] =
            controller.material_texture_if_ready(r, path, "render.vfx.project_texture");
    }
    match controller.gpu.vfx_particles.record_frame(
        r,
        scene.world(),
        controller.frame.frame_index,
        scope.dt,
        extraction.viewproj,
        view_matrix,
        extraction.camera_position,
        scene_color_format,
        scope.vp_w,
        scope.vp_h,
        vfx_texture_slots,
    ) {
        Ok(report) => {
            if scope.trace_frame && report.high_water > 0 {
                newengine_ulog_api::ulog::debug!(
                    "vfx gpu particles: high_water={} uploaded={} killed={} capacity_drops={}",
                    report.high_water,
                    report.uploaded_spawns,
                    report.killed_particles,
                    report.capacity_drops,
                );
            }
        }
        Err(error) if is_transient_shader_pipeline_error(&error) => {
            newengine_ulog_api::ulog::debug!(
                "vfx gpu particles: shader/pipeline not ready; semantic GPU spawns remain queued: {}",
                error
            );
        }
        Err(error) => {
            newengine_ulog_api::ulog::warn!(
                "vfx gpu particles: frame realization skipped without disabling scene rendering: {}",
                error
            );
        }
    }
}

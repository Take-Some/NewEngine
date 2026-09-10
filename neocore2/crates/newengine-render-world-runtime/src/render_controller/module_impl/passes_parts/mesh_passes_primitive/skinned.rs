use super::plan::primitive_role_cull_reason;
use super::*;
use crate::render_controller::module_impl::passes::mesh_visibility::sphere_screen_coverage_hint;

/// Draws player-owned skinned primitive parts through a dedicated non-instanced
/// character path. Static/foliage batching deliberately excludes these entities.
pub(crate) fn draw_skinned_player_primitives(
    this: &mut RuntimeRenderController,
    r: &mut dyn newengine_core::render::RenderApi,
    scene: &newengine_scene::Scene,
    lit: newengine_material_domain_api::LitPipeline,
    pass: SceneMeshPass,
    viewproj: Mat4,
    lights: &PackedLights,
    shadow_texture: TextureId,
    local_shadow_texture: TextureId,
    runtime: bool,
    camera_position: Vec3,
    _camera_forward: Vec3,
    deferred: bool,
) -> newengine_core::EngineResult<()> {
    use crate::render_controller::gpu::{ensure_player_skin_gpu, ensure_skin_palette_gpu};

    let world = scene.world();
    let reg_lock = this.bridges.scene.primitives();
    let reg = reg_lock.read();
    let mats_lock = this.bridges.scene.materials();
    let mats = mats_lock.read();
    let visibility_settings = primitive_visibility_settings(runtime, viewproj);
    // Diagnostic gate for character alpha overlays. TLOU-derived character packages can
    // contain both alpha-blend and dither/cutout presentations of the same authored hair
    // or facial layer. This switch lets the runtime prove whether translucent overlays are
    // the source of face/body corruption without changing authored assets.
    let disable_skinned_alpha_blend =
        runtime && std::env::var_os("NEWENGINE_DEBUG_DISABLE_SKIN_ALPHA_BLEND").is_some();

    let force_skinned_double_sided =
        runtime && std::env::var_os("NEWENGINE_DEBUG_FORCE_SKIN_DOUBLE_SIDED").is_some();

    for (entity, prim, global) in world.query2::<Primitive, GlobalTransform>() {
        let Some(skin) =
            world.get::<newengine_gameplay_world_runtime::gameplay::PlayerSkinBinding>(entity)
        else {
            continue;
        };
        if !display_visible_in_mode(world, entity, runtime) {
            continue;
        }
        let render_model = newengine_gameplay_world_runtime::gameplay::player_render_model_matrix(
            world, entity, global.0,
        );
        // Skin ownership is palette ownership, not character-model ownership. Equipped long guns
        // own an independent PlayerSkinPose on their weapon root and intentionally do not carry a
        // PlayerModelBinding. Requiring PlayerModelBinding here made their shadow path render while
        // the forward path silently discarded every skinned weapon primitive.
        let Some(pose) =
            world.get::<newengine_gameplay_world_runtime::gameplay::PlayerSkinPose>(skin.owner)
        else {
            continue;
        };
        if pose.palette.is_empty() {
            continue;
        }

        let transformed_bounds = world.get::<Bounds>(entity).map(|bounds| {
            transform_sphere(
                render_model,
                bounds.local_sphere.center,
                bounds.local_sphere.radius,
            )
        });
        let fallback_distance_m = distance_sq_to_camera(render_model, camera_position).sqrt();
        let (distance_m, screen_coverage) = transformed_bounds
            .map(|(center_ws, radius_ws)| {
                let distance = (center_ws - camera_position).length();
                (distance, sphere_screen_coverage_hint(radius_ws, distance))
            })
            .unwrap_or_else(|| {
                (
                    fallback_distance_m,
                    sphere_screen_coverage_hint(1.0, fallback_distance_m),
                )
            });
        if runtime && visibility_settings.culling_enabled {
            if let Some((center_ws, radius_ws)) = transformed_bounds {
                if !frustum_sphere_visible(
                    &visibility_settings.frustum,
                    camera_position,
                    center_ws,
                    radius_ws,
                    visibility_settings.max_distance,
                    visibility_settings.near_accept_distance,
                ) {
                    continue;
                }
            }
        }

        let gpu = ensure_primitive_gpu(&reg, prim.id, &mut this.gpu.meshes.prim_cache, r)?;
        let skin_gpu = ensure_player_skin_gpu(
            &mut this.gpu.meshes.skin_vertex_cache,
            prim.id,
            gpu,
            skin,
            r,
        )?;
        if skin_gpu.max_joint_index as usize >= pose.palette.len() {
            return Err(newengine_core::EngineError::other(format!(
                "skinned draw joint index outside palette entity={} primitive={} max_joint={} palette_joints={}",
                entity.stable_u64(),
                prim.id.0,
                skin_gpu.max_joint_index,
                pose.palette.len(),
            )));
        }
        let pose_generation = world
            .get::<newengine_gameplay_world_runtime::gameplay::PlayerModelBinding>(skin.owner)
            .map(|binding| binding.assignment_revision)
            .unwrap_or(0);
        let palette_gpu = ensure_skin_palette_gpu(
            &mut this.gpu.meshes.skin_palette_cache,
            &mut this.gpu.lifetimes.resources,
            skin.owner.stable_u64(),
            pose_generation,
            pose,
            lit.skin_bgl,
            this.frame.frame_index,
            this.backend_execution.host_visible_ring_slots(),
            r,
        )?;

        let material_ref = world
            .get::<newengine_materials::MaterialRef>(entity)
            .copied();
        let resolved = material_ref.and_then(|reference| mats.resolve(reference.id));
        let material_plan = LitMaterialPlan::from_resolved(resolved.as_ref(), prim.color);
        let render_options = world
            .get::<MeshRenderOptions>(entity)
            .cloned()
            .unwrap_or_else(MeshRenderOptions::character_body);
        let forward_alpha_surface = matches!(pass, SceneMeshPass::Forward)
            && (material_plan.alpha_blend || material_plan.alpha_cutoff > 0.0);
        if !forward_alpha_surface
            && primitive_role_cull_reason(
                &render_options,
                pass,
                this.runtime_profile().draw_sky_visuals(),
                deferred,
            )
            .is_some()
        {
            continue;
        }
        let player_visual =
            world.get::<newengine_gameplay_world_runtime::gameplay::PlayerVisualPart>(entity);
        let equipped_weapon = player_visual.is_some_and(|part| {
            part.kind
                == newengine_gameplay_world_runtime::gameplay::PlayerVisualKind::EquippedWeapon
        });
        this.request_material_set_with_view_hints(
            material_plan.base_color_texture,
            material_plan.normal_texture,
            material_plan.roughness_texture,
            screen_coverage,
            distance_m,
            if equipped_weapon { u8::MAX } else { 224 },
            equipped_weapon,
        );
        if disable_skinned_alpha_blend
            && matches!(pass, SceneMeshPass::Forward)
            && material_plan.alpha_blend
        {
            continue;
        }
        // Deferred GBuffer is opaque-only. Transparent overlays and authored
        // alpha-cutout surfaces stay in forward, which already consumes the exact
        // material cutoff instead of guessing from texture alpha.
        if pass.is_gbuffer() && (material_plan.alpha_blend || material_plan.alpha_cutoff > 0.0) {
            continue;
        }
        let base_texture = if let Some(path) = material_plan.base_color_texture {
            let Some(texture) = this.material_texture_if_ready(r, path, "render.skinned_character")
            else {
                // A declared character albedo is semantic content, not an optional detail.
                // Drawing it with the generic white texture turns skin/eyes into a grey PBR
                // fallback and hides residency failures. Omit this part until the authored
                // base texture is genuinely resident; neutral normal/roughness fallbacks are
                // still safe below because they do not replace the character's color identity.
                continue;
            };
            texture
        } else {
            lit.white_texture
        };
        let normal_texture = this.material_texture_or_default(
            r,
            material_plan.normal_texture,
            lit.flat_normal_texture,
        );
        let roughness_texture =
            this.material_texture_or_default(r, material_plan.roughness_texture, lit.white_texture);
        let sampler = if material_plan.alpha_cutoff > 0.0 {
            lit.clamp_sampler
        } else if material_plan.has_textures() {
            lit.repeat_sampler
        } else {
            lit.clamp_sampler
        };
        let pipeline = match pass {
            SceneMeshPass::Forward
                if material_plan.alpha_blend
                    && (material_plan.double_sided || force_skinned_double_sided) =>
            {
                lit.skinned_alpha_double_sided_pipeline
            }
            SceneMeshPass::Forward if material_plan.alpha_blend => lit.skinned_alpha_pipeline,
            SceneMeshPass::Forward if material_plan.double_sided || force_skinned_double_sided => {
                lit.skinned_double_sided_pipeline
            }
            SceneMeshPass::Forward => lit.skinned_pipeline,
            SceneMeshPass::GBuffer if material_plan.double_sided || force_skinned_double_sided => {
                lit.gbuffer_skinned_double_sided_pipeline
            }
            SceneMeshPass::GBuffer => lit.gbuffer_skinned_pipeline,
        };
        let receive_shadow_texture = if material_plan.receive_shadows {
            shadow_texture
        } else {
            lit.white_texture
        };
        let receive_local_shadow_texture =
            if matches!(pass, SceneMeshPass::Forward) && material_plan.receive_shadows {
                local_shadow_texture
            } else {
                lit.white_texture
            };
        // Per-draw UBOs are host-visible and may still be read by an in-flight frame.
        // The central per-draw cache adds the physical frame slot. Keep the logical
        // identity stable so callers cannot accidentally multiply that ring.
        let ubo_key = instance_batch_ubo_key(
            0x736b_696e_0000_0000 ^ entity.stable_u64() ^ prim.id.0,
            pipeline,
            base_texture,
            normal_texture,
            roughness_texture,
            receive_shadow_texture,
            receive_local_shadow_texture,
            sampler,
        );
        let per = this.ensure_per_draw_ubo_with_binding(
            r,
            lit,
            ubo_key,
            base_texture,
            normal_texture,
            roughness_texture,
            receive_shadow_texture,
            receive_local_shadow_texture,
            sampler,
        )?;
        crate::render_controller::module_impl::passes_ubo::write_lit_ubo_ex(
            r,
            per.ubo,
            viewproj * render_model,
            render_model,
            material_plan.base_color,
            material_plan.emissive_radiance,
            material_plan.alpha_cutoff,
            material_plan.uv_transform,
            material_plan.material_params,
            lights,
        )?;

        r.set_pipeline(pipeline)?;
        r.set_bind_group(0, per.bg)?;
        r.set_bind_group(1, palette_gpu.bg)?;
        r.set_vertex_buffer(0, BufferSlice::new(gpu.vb, 0))?;
        r.set_vertex_buffer(1, BufferSlice::new(skin_gpu.vb, 0))?;
        r.set_index_buffer(BufferSlice::new(gpu.ib, 0), IndexFormat::U32)?;
        r.draw_indexed(DrawIndexedArgs::new(gpu.index_count))?;
        this.diagnostics
            .overlay_metrics
            .record_indexed_triangles(gpu.index_count);
    }
    Ok(())
}

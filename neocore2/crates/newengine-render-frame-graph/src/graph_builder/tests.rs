use super::*;
use crate::{standard_runtime_frame, StandardRuntimePipelineDesc};
use newengine_render_api::{
    Extent2D, RenderGraphPassKind, RenderGraphQueueKind, RenderGraphResourceUsage, RenderTargetId,
    TextureFormat,
};

#[test]
fn deferred_runtime_graph_uses_explicit_lighting_resolve_without_forward_replay() {
    let plan = standard_runtime_frame(
        StandardRuntimePipelineDesc::new(7, Extent2D::new(1280, 720), Extent2D::new(1280, 720))
            .deferred(true)
            .postfx(false)
            .ui(false)
            .debug_overlay(false),
    );
    let graph = plan.graph;
    let gbuffer_index = graph
        .passes
        .iter()
        .position(|pass| pass.kind == RenderGraphPassKind::GBuffer)
        .expect("deferred graph must contain a GBuffer pass");
    let lighting_index = graph
        .passes
        .iter()
        .position(|pass| pass.kind == RenderGraphPassKind::DeferredLighting)
        .expect("deferred graph must contain a DeferredLighting resolve pass");
    assert!(
        gbuffer_index < lighting_index,
        "DeferredLighting must execute after GBuffer"
    );

    let lighting = &graph.passes[lighting_index];
    assert!(
        lighting.draw_lists.is_empty(),
        "DeferredLighting is a fullscreen resolve pass and must not replay OpaqueForward draw lists"
    );
    for resource in [
        RG_GBUFFER_ALBEDO,
        RG_GBUFFER_NORMAL,
        RG_GBUFFER_MATERIAL,
        RG_GBUFFER_DEPTH,
    ] {
        assert!(
            lighting.reads.iter().any(|read| read.resource == resource
                && read.usage == RenderGraphResourceUsage::SampledTexture),
            "DeferredLighting must sample GBuffer resource {:?}",
            resource
        );
    }
    assert!(
        lighting
            .writes
            .iter()
            .any(|write| write.resource == RG_SCENE_HDR_COLOR
                && write.usage == RenderGraphResourceUsage::ColorAttachment),
        "DeferredLighting must resolve into canonical scene color"
    );
    assert!(!lighting
        .writes
        .iter()
        .any(|write| write.resource == RG_LIT_COLOR));
}

#[test]
fn deferred_runtime_routes_opaque_geometry_into_gbuffer() {
    let plan = standard_runtime_frame(
        StandardRuntimePipelineDesc::new(18, Extent2D::new(1280, 720), Extent2D::new(1280, 720))
            .deferred(true)
            .postfx(false)
            .ui(false)
            .debug_overlay(false)
            .draw_lists([crate::DrawListDesc::standard(
                newengine_render_api::RenderDrawListKind::OpaqueForward,
            )]),
    );

    let gbuffer = plan
        .graph
        .passes
        .iter()
        .find(|pass| pass.kind == RenderGraphPassKind::GBuffer)
        .expect("deferred graph must contain a GBuffer pass");
    assert!(
        gbuffer
            .draw_lists
            .contains(&newengine_render_api::RenderDrawListKind::OpaqueForward),
        "GBuffer must consume OpaqueForward geometry in deferred mode"
    );

    let report = plan.validate_draw_list_routes();
    assert!(
        report.errors.is_empty(),
        "deferred draw-list routes must validate: {:?}",
        report.errors
    );
    assert!(
        !report
            .warnings
            .iter()
            .any(|issue| issue.code == "draw_list.multiple_routes"),
        "GBuffer + ForwardOpaque is an intentional phase-partitioned OpaqueForward route in deferred mode"
    );
}

#[test]
fn deferred_gbuffer_formats_match_material_domain_mrt_contract() {
    let plan = standard_runtime_frame(
        StandardRuntimePipelineDesc::new(20, Extent2D::new(1280, 720), Extent2D::new(1280, 720))
            .deferred(true)
            .postfx(false)
            .ui(false)
            .debug_overlay(false),
    );

    let format_for = |id| {
        plan.graph
            .resources
            .iter()
            .find(|resource| resource.id == id)
            .and_then(|resource| resource.format)
            .expect("GBuffer resource must declare a format")
    };

    assert_eq!(format_for(RG_GBUFFER_ALBEDO), TextureFormat::Rgba8Unorm);
    assert_eq!(format_for(RG_GBUFFER_NORMAL), TextureFormat::Rgba16Float);
    assert_eq!(format_for(RG_GBUFFER_MATERIAL), TextureFormat::Rgba8Unorm);
    assert_eq!(format_for(RG_GBUFFER_DEPTH), TextureFormat::Depth32Float);
}

#[test]
fn deferred_runtime_separates_gbuffer_and_scene_depth_ownership() {
    let plan = standard_runtime_frame(
        StandardRuntimePipelineDesc::new(19, Extent2D::new(1280, 720), Extent2D::new(1280, 720))
            .deferred(true)
            .hdr_scene(true)
            .postfx(true)
            .ui(false)
            .debug_overlay(false)
            .draw_lists([crate::DrawListDesc::standard(
                newengine_render_api::RenderDrawListKind::OpaqueForward,
            )]),
    );

    assert!(
        !plan
            .graph
            .passes
            .iter()
            .any(|pass| pass.kind == RenderGraphPassKind::DepthPrepass),
        "deferred graph must not duplicate opaque depth outside the GBuffer"
    );

    let gbuffer = plan
        .graph
        .passes
        .iter()
        .find(|pass| pass.kind == RenderGraphPassKind::GBuffer)
        .expect("deferred graph must contain GBuffer");
    assert!(gbuffer.writes.iter().any(|write| {
        write.resource == RG_GBUFFER_DEPTH
            && write.usage == RenderGraphResourceUsage::DepthAttachment
    }));
    assert!(!gbuffer
        .reads
        .iter()
        .any(|read| read.resource == RG_VIEWPORT_DEPTH));

    let transparent = plan
        .graph
        .passes
        .iter()
        .find(|pass| pass.kind == RenderGraphPassKind::Transparent)
        .expect("deferred graph must contain Transparent");
    assert!(transparent.reads.iter().any(|read| {
        read.resource == RG_GBUFFER_DEPTH
            && read.usage == RenderGraphResourceUsage::DepthAttachmentSampled
    }));
    assert!(!transparent
        .reads
        .iter()
        .any(|read| read.resource == RG_VIEWPORT_DEPTH));

    let postfx = plan
        .graph
        .passes
        .iter()
        .find(|pass| pass.kind == RenderGraphPassKind::PostFx)
        .expect("deferred HDR graph must contain PostFx");
    assert!(postfx.reads.iter().any(|read| {
        read.resource == RG_GBUFFER_DEPTH && read.usage == RenderGraphResourceUsage::SampledTexture
    }));
    assert!(!postfx
        .reads
        .iter()
        .any(|read| read.resource == RG_VIEWPORT_DEPTH));
    assert!(postfx
        .reads
        .iter()
        .any(|read| read.resource == RG_SCENE_HDR_COLOR));

    newengine_render_api::compile_render_graph(&plan.graph)
        .expect("deferred depth domains must have independent producers");
}

#[test]
fn forward_runtime_graph_does_not_emit_deferred_resolve() {
    let plan = standard_runtime_frame(
        StandardRuntimePipelineDesc::new(8, Extent2D::new(640, 360), Extent2D::new(640, 360))
            .deferred(false)
            .postfx(false)
            .ui(false)
            .debug_overlay(false),
    );
    assert!(
        !plan
            .graph
            .passes
            .iter()
            .any(|pass| pass.kind == RenderGraphPassKind::DeferredLighting),
        "forward graph must not emit deferred lighting resolve"
    );
}

#[test]
fn hdr_scene_with_postfx_writes_viewport_through_postfx() {
    let plan = standard_runtime_frame(
        StandardRuntimePipelineDesc::new(9, Extent2D::new(1280, 720), Extent2D::new(1280, 720))
            .hdr_scene(true)
            .postfx(true)
            .ui(false)
            .debug_overlay(false),
    );

    let postfx = plan
        .graph
        .passes
        .iter()
        .find(|pass| pass.kind == RenderGraphPassKind::PostFx)
        .expect("HDR scene with postFX must emit PostFx surface resolve");
    assert!(postfx
        .reads
        .iter()
        .any(|read| read.resource == RG_SCENE_HDR_COLOR
            && read.usage == RenderGraphResourceUsage::SampledTexture));
    assert!(postfx
        .reads
        .iter()
        .any(|read| read.resource == RG_VIEWPORT_DEPTH
            && read.usage == RenderGraphResourceUsage::SampledTexture),
        "postFX micro-visibility must consume the scene depth without adding a second depth prepass");
    assert!(postfx
        .writes
        .iter()
        .any(|write| write.resource == RG_VIEWPORT_COLOR
            && write.usage == RenderGraphResourceUsage::ColorAttachment));
    assert!(!plan
        .graph
        .passes
        .iter()
        .any(|pass| pass.kind == RenderGraphPassKind::Copy
            && pass.label == "hdr_scene_resolve_to_surface"));
}

#[test]
fn hdr_scene_without_postfx_adds_viewport_resolve() {
    let plan = standard_runtime_frame(
        StandardRuntimePipelineDesc::new(10, Extent2D::new(1280, 720), Extent2D::new(1280, 720))
            .hdr_scene(true)
            .postfx(false)
            .ui(false)
            .debug_overlay(false),
    );

    let resolve = plan
        .graph
        .passes
        .iter()
        .find(|pass| {
            pass.kind == RenderGraphPassKind::Copy && pass.label == "hdr_scene_resolve_to_surface"
        })
        .expect("HDR scene without postFX must still resolve scene color into the surface");
    assert!(resolve
        .reads
        .iter()
        .any(|read| read.resource == RG_SCENE_HDR_COLOR
            && read.usage == RenderGraphResourceUsage::SampledTexture));
    assert!(resolve
        .writes
        .iter()
        .any(|write| write.resource == RG_VIEWPORT_COLOR
            && write.usage == RenderGraphResourceUsage::ColorAttachment));
}

#[test]
fn ui_only_composite_writes_surface_without_scene_resolve() {
    let plan = standard_runtime_frame(
        StandardRuntimePipelineDesc::new(11, Extent2D::new(800, 600), Extent2D::new(800, 600))
            .hdr_scene(false)
            .postfx(false)
            .ui(true)
            .debug_overlay(false),
    );

    assert!(plan
        .graph
        .passes
        .iter()
        .any(|pass| pass.kind == RenderGraphPassKind::UiComposite
            && pass
                .writes
                .iter()
                .any(|write| write.resource == RG_SURFACE_COLOR)));
    assert!(!plan
        .graph
        .passes
        .iter()
        .any(|pass| pass.kind == RenderGraphPassKind::Copy
            && pass.label == "hdr_scene_resolve_to_surface"));
}

#[test]
fn debug_overlay_runs_after_ui_composite() {
    let plan = standard_runtime_frame(
        StandardRuntimePipelineDesc::new(12, Extent2D::new(800, 600), Extent2D::new(800, 600))
            .postfx(false)
            .ui(true)
            .debug_overlay(true),
    );
    let ui = plan
        .graph
        .passes
        .iter()
        .position(|pass| pass.kind == RenderGraphPassKind::UiComposite)
        .expect("UI composite pass must exist");
    let debug = plan
        .graph
        .passes
        .iter()
        .position(|pass| pass.kind == RenderGraphPassKind::DebugOverlay)
        .expect("Debug overlay pass must exist");
    assert!(ui < debug, "Debug overlay must execute after UI composite");
}

#[test]
fn shadow_cascade_atlas_scales_with_cascade_count() {
    let plan = standard_runtime_frame(
        StandardRuntimePipelineDesc::new(13, Extent2D::new(800, 600), Extent2D::new(800, 600))
            .shadow(true, 1024)
            .shadow_cascades(5)
            .postfx(false)
            .ui(false)
            .debug_overlay(false),
    );
    let shadow = plan
        .graph
        .resources
        .iter()
        .find(|resource| resource.id == RG_SHADOW_MAP)
        .expect("cascade shadow map resource must exist");
    assert_eq!(shadow.extent, Some(Extent2D::new(4096, 2048)));
}

#[test]
fn draw_list_route_validation_warns_when_pass_route_has_no_declared_list() {
    let mut plan = standard_runtime_frame(
        StandardRuntimePipelineDesc::new(14, Extent2D::new(800, 600), Extent2D::new(800, 600))
            .hdr_scene(false)
            .postfx(false)
            .ui(true)
            .debug_overlay(false),
    );
    // Standard runtime construction normalizes graph-owned routes. Remove one
    // declaration deliberately so this test remains a validator-level negative case.
    plan.draw_lists
        .retain(|desc| desc.kind != newengine_render_api::RenderDrawListKind::Transparent);

    let report = plan.validate_draw_list_routes();
    assert!(
        report.ok,
        "missing declared route is a warning, not a hard graph error"
    );
    assert!(report
        .warnings
        .iter()
        .any(|issue| issue.code == "draw_list.route_without_declared_list"));
}

#[test]
fn external_viewport_target_is_declared_when_requested() {
    let target = RenderTargetId(std::num::NonZeroU32::new(77).unwrap());
    let plan = standard_runtime_frame(
        StandardRuntimePipelineDesc::new(15, Extent2D::new(1280, 720), Extent2D::new(640, 360))
            .hdr_scene(false)
            .viewport_render_target(Some(target))
            .postfx(false)
            .ui(false)
            .debug_overlay(false),
    );

    let viewport = plan
        .graph
        .resources
        .iter()
        .find(|resource| resource.id == RG_VIEWPORT_COLOR)
        .expect("external viewport color resource must exist");
    assert_eq!(
        viewport.external,
        Some(newengine_render_api::RenderGraphExternalResource::RenderTarget(target))
    );
}

#[test]
fn viewport_surface_with_hdr_scene_uses_offscreen_scene_color_and_depth() {
    let plan = standard_runtime_frame(
        StandardRuntimePipelineDesc::new(16, Extent2D::new(1280, 720), Extent2D::new(1280, 720))
            .viewport_is_surface(true)
            .hdr_scene(true)
            .postfx(false)
            .ui(false)
            .debug_overlay(false),
    );

    assert!(plan
        .graph
        .resources
        .iter()
        .any(|resource| resource.id == RG_SCENE_HDR_COLOR && resource.external.is_none()));
    let depth = plan
        .graph
        .resources
        .iter()
        .find(|resource| resource.id == RG_VIEWPORT_DEPTH)
        .expect("HDR scene must use offscreen scene depth");
    assert!(depth.external.is_none());
    assert!(plan
        .graph
        .passes
        .iter()
        .any(|pass| pass.kind == RenderGraphPassKind::Copy
            && pass.label == "hdr_scene_resolve_to_surface"));
}

#[test]
fn hair_simulation_storage_dependency_reaches_transparent_pass() {
    let plan = standard_runtime_frame(
        StandardRuntimePipelineDesc::new(17, Extent2D::new(1280, 720), Extent2D::new(1280, 720))
            .hair(true)
            .postfx(false)
            .ui(false)
            .debug_overlay(false),
    );

    let hair = plan
        .graph
        .passes
        .iter()
        .find(|pass| pass.kind == RenderGraphPassKind::HairSimulation)
        .expect("Hair-enabled graph must contain HairSimulation");
    assert_eq!(hair.queue, RenderGraphQueueKind::Compute);
    let state = hair
        .writes
        .iter()
        .find(|write| write.usage == RenderGraphResourceUsage::StorageBuffer)
        .expect("HairSimulation must publish strand state")
        .resource;

    let transparent = plan
        .graph
        .passes
        .iter()
        .find(|pass| pass.kind == RenderGraphPassKind::Transparent)
        .expect("Hair-enabled graph must retain Transparent raster");
    assert!(transparent.reads.iter().any(|read| {
        read.resource == state && read.usage == RenderGraphResourceUsage::StorageBuffer
    }));

    newengine_render_api::compile_render_graph(&plan.graph)
        .expect("Hair compute-to-transparent graph must compile as a valid dependency DAG");
}

#[test]
fn hair_state_flows_through_csm_before_transparent() {
    let plan = standard_runtime_frame(
        StandardRuntimePipelineDesc::new(23, Extent2D::new(1280, 720), Extent2D::new(1280, 720))
            .hair(true)
            .shadow(true, 1024)
            .shadow_cascades(4)
            .postfx(false)
            .ui(false)
            .debug_overlay(false),
    );
    let hair = plan
        .graph
        .passes
        .iter()
        .find(|pass| pass.kind == RenderGraphPassKind::HairSimulation)
        .expect("HairSimulation");
    let hair_state = hair
        .writes
        .iter()
        .find(|write| write.usage == RenderGraphResourceUsage::StorageBuffer)
        .expect("hair state")
        .resource;
    let csm = plan
        .graph
        .passes
        .iter()
        .find(|pass| pass.kind == RenderGraphPassKind::ShadowCascadeMap)
        .expect("ShadowCascadeMap");
    assert!(csm.reads.iter().any(|read| {
        read.resource == hair_state && read.usage == RenderGraphResourceUsage::StorageBuffer
    }));
    let shadow_map = csm
        .writes
        .iter()
        .find(|write| write.usage == RenderGraphResourceUsage::ColorAttachment)
        .expect("CSM color-packed depth")
        .resource;
    let transparent = plan
        .graph
        .passes
        .iter()
        .find(|pass| pass.kind == RenderGraphPassKind::Transparent)
        .expect("Transparent");
    assert!(transparent.reads.iter().any(|read| {
        read.resource == shadow_map && read.usage == RenderGraphResourceUsage::SampledTexture
    }));

    let compiled = newengine_render_api::compile_render_graph(&plan.graph)
        .expect("HairSimulation -> CSM -> Transparent dependency DAG must compile");
    let hair_pos = compiled
        .execution_order
        .iter()
        .position(|id| *id == hair.id)
        .unwrap();
    let csm_pos = compiled
        .execution_order
        .iter()
        .position(|id| *id == csm.id)
        .unwrap();
    let transparent_pos = compiled
        .execution_order
        .iter()
        .position(|id| *id == transparent.id)
        .unwrap();
    assert!(hair_pos < csm_pos && csm_pos < transparent_pos);
}

#[test]
fn deferred_forward_overlay_consumes_forward_only_bucket_before_postfx() {
    let plan = standard_runtime_frame(
        StandardRuntimePipelineDesc::new(
            91,
            newengine_render_api::Extent2D::new(1280, 720),
            newengine_render_api::Extent2D::new(1280, 720),
        )
        .viewport_is_surface(true)
        .deferred(true)
        .postfx(true)
        .ui(false)
        .debug_overlay(false),
    );

    let gbuffer = plan
        .graph
        .passes
        .iter()
        .position(|pass| pass.kind == newengine_render_api::RenderGraphPassKind::GBuffer)
        .expect("deferred frame must contain GBuffer");
    let lighting = plan
        .graph
        .passes
        .iter()
        .position(|pass| pass.kind == newengine_render_api::RenderGraphPassKind::DeferredLighting)
        .expect("deferred frame must contain DeferredLighting");
    let overlay = plan
        .graph
        .passes
        .iter()
        .position(|pass| pass.kind == newengine_render_api::RenderGraphPassKind::ForwardOpaque)
        .expect("deferred frame must contain forward overlay for sky/view-model roles");
    let transparent = plan
        .graph
        .passes
        .iter()
        .position(|pass| pass.kind == newengine_render_api::RenderGraphPassKind::Transparent)
        .expect("transparent pass");
    let postfx = plan
        .graph
        .passes
        .iter()
        .position(|pass| pass.kind == newengine_render_api::RenderGraphPassKind::PostFx)
        .expect("postfx pass");

    assert!(
        gbuffer < lighting && lighting < overlay && overlay < transparent && transparent < postfx
    );

    let pass = &plan.graph.passes[overlay];
    assert!(pass
        .draw_lists
        .contains(&newengine_render_api::RenderDrawListKind::OpaqueForward));
    assert!(pass
        .writes
        .iter()
        .any(|access| access.resource == crate::RG_SCENE_HDR_COLOR));
    assert!(pass
        .reads
        .iter()
        .any(|access| access.resource == crate::RG_GBUFFER_DEPTH
            && access.usage == newengine_render_api::RenderGraphResourceUsage::DepthAttachment));
    assert!(!pass
        .writes
        .iter()
        .any(|access| access.resource == crate::RG_VIEWPORT_DEPTH));
}

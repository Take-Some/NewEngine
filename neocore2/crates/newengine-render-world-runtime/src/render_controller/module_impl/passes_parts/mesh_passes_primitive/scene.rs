use super::plan::*;

use super::*;

#[path = "scene_editor_overlay.rs"]
mod editor_overlay;
#[path = "scene_pass.rs"]
mod pass;
#[path = "scene_wireframe.rs"]
mod wireframe;

use editor_overlay::draw_editor_viewport_overlays;
use pass::{draw_primitives_for_pass, PrimitivePassSlice};
use wireframe::draw_primitives_wireframe;

pub fn draw_primitives(
    this: &mut RuntimeRenderController,
    r: &mut dyn newengine_core::render::RenderApi,
    scene: &newengine_scene::Scene,
    lit: newengine_material_domain_api::LitPipeline,
    viewproj: Mat4,
    lights: &PackedLights,
    shadow_texture: TextureId,
    local_shadow_texture: TextureId,
    runtime: bool,
    camera_position: Vec3,
    camera_forward: Vec3,
    deferred: bool,
) -> newengine_core::EngineResult<()> {
    if this.editor_viewport.is_active()
        && this.editor_viewport.shading() == newengine_ui_api::UiEditorViewportShading::Wireframe
    {
        draw_primitives_wireframe(this, r, scene, viewproj, runtime)?;
        super::draw_model_components_wireframe(this, r, scene, viewproj, runtime)?;
        return draw_editor_viewport_overlays(this, r, scene, viewproj);
    }
    let stage_profile = runtime
        && (this.frame.frame_index <= 3 || this.frame.frame_index.is_multiple_of(30))
        && newengine_runtime_policy::render_runtime_policy().primitive_stage_log;
    let stage_total = stage_profile.then(std::time::Instant::now);
    let started = stage_profile.then(std::time::Instant::now);
    draw_primitives_for_pass(
        this,
        r,
        scene,
        lit,
        SceneMeshPass::Forward,
        viewproj,
        lights,
        shadow_texture,
        local_shadow_texture,
        runtime,
        camera_position,
        camera_forward,
        deferred,
        PrimitivePassSlice::NonDecal,
    )?;
    let static_ms = started
        .map(|v| v.elapsed().as_secs_f64() * 1000.0)
        .unwrap_or(0.0);
    let started = stage_profile.then(std::time::Instant::now);
    super::draw_skinned_player_primitives(
        this,
        r,
        scene,
        lit,
        SceneMeshPass::Forward,
        viewproj,
        lights,
        shadow_texture,
        local_shadow_texture,
        runtime,
        camera_position,
        camera_forward,
        deferred,
    )?;
    let skinned_ms = started
        .map(|v| v.elapsed().as_secs_f64() * 1000.0)
        .unwrap_or(0.0);
    let started = stage_profile.then(std::time::Instant::now);
    super::draw_model_components(
        this,
        r,
        scene,
        lit,
        SceneMeshPass::Forward,
        viewproj,
        lights,
        shadow_texture,
        local_shadow_texture,
        runtime,
        camera_position,
        camera_forward,
        deferred,
    )?;
    let models_ms = started
        .map(|v| v.elapsed().as_secs_f64() * 1000.0)
        .unwrap_or(0.0);
    let started = stage_profile.then(std::time::Instant::now);
    // Decals are forward overlays. Replaying them before model/YDD world geometry lets later opaque
    // batches overwrite their color even though the decal pipeline itself is depth-read-only.
    // Keep the overlay partition physically after all opaque/skinned/model world draws.
    draw_primitives_for_pass(
        this,
        r,
        scene,
        lit,
        SceneMeshPass::Forward,
        viewproj,
        lights,
        shadow_texture,
        local_shadow_texture,
        runtime,
        camera_position,
        camera_forward,
        deferred,
        PrimitivePassSlice::DecalOnly,
    )?;
    let decals_ms = started
        .map(|v| v.elapsed().as_secs_f64() * 1000.0)
        .unwrap_or(0.0);
    let started = stage_profile.then(std::time::Instant::now);
    let result = draw_editor_viewport_overlays(this, r, scene, viewproj);
    let overlays_ms = started
        .map(|v| v.elapsed().as_secs_f64() * 1000.0)
        .unwrap_or(0.0);
    if stage_profile {
        newengine_ulog_api::ulog::info!(
            "primitive.provider.stage.profile: frame={} pass='forward' total_ms={:.3} static_ms={:.3} skinned_ms={:.3} models_ms={:.3} decals_ms={:.3} overlays_ms={:.3}",
            this.frame.frame_index,
            stage_total.map(|v| v.elapsed().as_secs_f64() * 1000.0).unwrap_or(0.0),
            static_ms, skinned_ms, models_ms, decals_ms, overlays_ms
        );
    }
    result
}

pub fn draw_primitives_gbuffer(
    this: &mut RuntimeRenderController,
    r: &mut dyn newengine_core::render::RenderApi,
    scene: &newengine_scene::Scene,
    lit: newengine_material_domain_api::LitPipeline,
    viewproj: Mat4,
    lights: &PackedLights,
    runtime: bool,
    camera_position: Vec3,
    camera_forward: Vec3,
    deferred: bool,
) -> newengine_core::EngineResult<()> {
    if this.editor_viewport.is_active()
        && this.editor_viewport.shading() == newengine_ui_api::UiEditorViewportShading::Wireframe
    {
        return Ok(());
    }
    let stage_profile = runtime
        && (this.frame.frame_index <= 3 || this.frame.frame_index.is_multiple_of(30))
        && newengine_runtime_policy::render_runtime_policy().primitive_stage_log;
    let stage_total = stage_profile.then(std::time::Instant::now);
    let started = stage_profile.then(std::time::Instant::now);
    draw_primitives_for_pass(
        this,
        r,
        scene,
        lit,
        SceneMeshPass::GBuffer,
        viewproj,
        lights,
        lit.white_texture,
        lit.white_texture,
        runtime,
        camera_position,
        camera_forward,
        deferred,
        PrimitivePassSlice::All,
    )?;
    let static_ms = started
        .map(|v| v.elapsed().as_secs_f64() * 1000.0)
        .unwrap_or(0.0);
    let started = stage_profile.then(std::time::Instant::now);
    super::draw_skinned_player_primitives(
        this,
        r,
        scene,
        lit,
        SceneMeshPass::GBuffer,
        viewproj,
        lights,
        lit.white_texture,
        lit.white_texture,
        runtime,
        camera_position,
        camera_forward,
        deferred,
    )?;
    let skinned_ms = started
        .map(|v| v.elapsed().as_secs_f64() * 1000.0)
        .unwrap_or(0.0);
    let started = stage_profile.then(std::time::Instant::now);
    let result = super::draw_model_components(
        this,
        r,
        scene,
        lit,
        SceneMeshPass::GBuffer,
        viewproj,
        lights,
        lit.white_texture,
        lit.white_texture,
        runtime,
        camera_position,
        camera_forward,
        deferred,
    );
    let models_ms = started
        .map(|v| v.elapsed().as_secs_f64() * 1000.0)
        .unwrap_or(0.0);
    if stage_profile {
        newengine_ulog_api::ulog::info!(
            "primitive.provider.stage.profile: frame={} pass='gbuffer' total_ms={:.3} static_ms={:.3} skinned_ms={:.3} models_ms={:.3}",
            this.frame.frame_index,
            stage_total.map(|v| v.elapsed().as_secs_f64() * 1000.0).unwrap_or(0.0),
            static_ms, skinned_ms, models_ms
        );
    }
    result
}

use crate::render_controller::module_impl::passes::mesh_visibility::sphere_screen_coverage_hint;
use crate::render_controller::state::MaterialTextureStreamingClass;
use newengine_core::render::{
    BindGroupId, BufferSlice, DrawIndexedArgs, IndexFormat, PipelineId, SamplerId, TextureId,
};
use newengine_math::{Mat4, Vec3};

use newengine_bounds::Bounds;
use newengine_materials::api::{MaterialId, MaterialRegistryApi};
use newengine_primitives::Primitive;
use newengine_transform::GlobalTransform;

use super::super::super::gpu::{ensure_primitive_gpu, upload_primitive_mesh, PrimitiveGpu};
use super::super::super::material_bindings::LitMaterialPlan;
use super::super::draw_bucket::{BucketedIndexedDrawStream, IndexedDrawPacket};
use super::super::instancing::{
    diagnostic_instance_token, draw_indexed_instanced_args, InstanceBatchKey, InstanceBatchSet,
    InstancedReplayState, RenderInstanceRaw,
};
use super::mesh_visibility::{
    distance_sq_to_camera, foliage_instance_budget, frustum_sphere_visible, primitive_budget,
    primitive_cast_shadows_enabled, primitive_shadow_max_distance, primitive_visibility_settings,
    render_scene_culling_enabled, shadow_caster_visible, sort_and_truncate_by_distance_then_key,
    sort_by_distance_then_key, sphere_within_render_distance, terrain_budget,
    terrain_cast_shadows_enabled, terrain_forward_max_distance, terrain_near_accept_distance,
    terrain_receive_shadows_enabled, transform_sphere,
};
use crate::render_controller::RuntimeRenderController;
use newengine_gameplay_world_runtime::gameplay::display_visible_in_mode;
use newengine_gameplay_world_runtime::gameplay::{
    EnvironmentDomeRenderState, TerrainMaterialLayers,
};
use newengine_math::collections::FxHashMap;
use newengine_model_domain_api::{
    MeshRenderOptions, MeshRenderRole, MeshShadowPolicy, MeshTransformPolicy,
};
use newengine_procedural_noise::ProceduralTerrain;
use newengine_render_feature_api::PackedLights;

mod mesh_passes_primitive;
mod mesh_passes_shadow;
mod scene_mesh_pass;

pub use self::mesh_passes_primitive::{
    draw_asset_preview_bundle, draw_primitives, draw_primitives_gbuffer,
};
pub(crate) use self::mesh_passes_shadow::ShadowUboViewKey;
pub use self::mesh_passes_shadow::{draw_primitives_shadow, draw_procedural_terrain_shadow};
use self::scene_mesh_pass::{route_diagnostics_due, SceneMeshPass};

pub(crate) fn publish_camera_spawn(
    bridge: &newengine_viewport_bridge::ViewportBridge,
    camera_position: Vec3,
    camera_forward: Vec3,
) {
    bridge.publish_camera_spawn(camera_position, camera_forward);
}

#[derive(Clone)]
struct TerrainDrawEntry {
    distance_sq: f32,
    screen_coverage: f32,
    entity_key: u64,
    mesh_key: u64,
    base_color: [f32; 4],
    model: Mat4,
    material: Option<newengine_materials::MaterialRef>,
    surface_layers: Option<TerrainMaterialLayers>,
    shadow_policy: MeshShadowPolicy,
}

#[derive(Clone)]
struct TerrainShadowEntry {
    entity_key: u64,
    mesh_key: u64,
    base_color: [f32; 4],
    bounds_center: Vec3,
    bounds_radius: f32,
    model: Mat4,
    material: Option<newengine_materials::MaterialRef>,
}

pub fn draw_procedural_terrain(
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
) -> newengine_core::EngineResult<()> {
    if this.editor_viewport.is_active()
        && this.editor_viewport.shading() == newengine_ui_api::UiEditorViewportShading::Wireframe
    {
        return draw_procedural_terrain_wireframe(this, r, scene, viewproj);
    }
    draw_procedural_terrain_for_pass(
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
    )
}

pub fn draw_procedural_terrain_gbuffer(
    this: &mut RuntimeRenderController,
    r: &mut dyn newengine_core::render::RenderApi,
    scene: &newengine_scene::Scene,
    lit: newengine_material_domain_api::LitPipeline,
    viewproj: Mat4,
    lights: &PackedLights,
    runtime: bool,
    camera_position: Vec3,
    camera_forward: Vec3,
) -> newengine_core::EngineResult<()> {
    if this.editor_viewport.is_active()
        && this.editor_viewport.shading() == newengine_ui_api::UiEditorViewportShading::Wireframe
    {
        // Terrain wire extraction is provider-owned; suppress the solid terrain pass
        // rather than mixing filled terrain into an otherwise wireframe viewport.
        return Ok(());
    }
    draw_procedural_terrain_for_pass(
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
    )
}

fn draw_procedural_terrain_for_pass(
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
) -> newengine_core::EngineResult<()> {
    let world = scene.world();
    let mats_lock = this.bridges.scene.materials();
    let mats = mats_lock.read();
    let terrain_culling_enabled = render_scene_culling_enabled();
    let terrain_max_distance = terrain_forward_max_distance();
    let terrain_frustum = (runtime && terrain_culling_enabled)
        .then(|| newengine_camera::Frustum::from_view_proj(viewproj));

    let mut entries: Vec<TerrainDrawEntry> = Vec::new();
    for (id, terrain, gt) in world.query2::<ProceduralTerrain, GlobalTransform>() {
        if !display_visible_in_mode(world, id, runtime) {
            continue;
        }
        let mesh_key = terrain.mesh_key();
        let local_bounds = terrain.heightfield.local_bounds();
        let (center_ws, radius_ws) = transform_sphere(
            gt.0,
            local_bounds.center(),
            local_bounds.half_extents().length(),
        );
        let distance_m = (center_ws - camera_position).length();
        let screen_coverage = sphere_screen_coverage_hint(radius_ws, distance_m);
        if runtime
            && !sphere_within_render_distance(
                camera_position,
                center_ws,
                radius_ws,
                terrain_max_distance,
            )
        {
            continue;
        }
        if runtime
            && terrain_culling_enabled
            && !frustum_sphere_visible(
                terrain_frustum.as_ref().expect("terrain culling frustum"),
                camera_position,
                center_ws,
                radius_ws,
                terrain_max_distance,
                terrain_near_accept_distance(radius_ws),
            )
        {
            continue;
        }
        let render_options = world
            .get::<MeshRenderOptions>(id)
            .cloned()
            .unwrap_or_else(MeshRenderOptions::terrain_patch);
        entries.push(TerrainDrawEntry {
            distance_sq: distance_m * distance_m,
            screen_coverage,
            entity_key: id.stable_u64(),
            mesh_key,
            base_color: terrain.base_color,
            model: gt.0,
            material: world.get::<newengine_materials::MaterialRef>(id).copied(),
            surface_layers: world.get::<TerrainMaterialLayers>(id).cloned(),
            shadow_policy: render_options.shadow_policy,
        });
    }
    entries.sort_by(|a, b| {
        a.distance_sq
            .partial_cmp(&b.distance_sq)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| a.entity_key.cmp(&b.entity_key))
    });
    let terrain_candidate_count = entries.len();
    let terrain_forward_budget = terrain_budget(runtime, false);
    entries.truncate(terrain_forward_budget);
    let terrain_planned_count = entries.len();

    let mut terrain_gpu_missing = 0usize;
    let mut terrain_submitted = 0usize;
    let mut terrain_shadow_bound = false;
    let mut stream = BucketedIndexedDrawStream::with_capacity(entries.len());
    for entry in entries {
        let entity_key = entry.entity_key;
        let distance_m = entry.distance_sq.sqrt();
        let screen_coverage = entry.screen_coverage;
        let mesh_key = entry.mesh_key;
        let model = entry.model;
        let material = entry.material;
        let surface_layers = entry.surface_layers;
        let shadow_policy = entry.shadow_policy;
        let Some(gpu) = this.gpu.meshes.terrain_cache.get(&mesh_key).copied() else {
            // Render residency is advanced by `pump_scene_gpu_residency` before
            // feature extraction. Missing GPU buffers are skipped for this frame
            // instead of being uploaded on the draw-list hot path.
            terrain_gpu_missing = terrain_gpu_missing.saturating_add(1);
            continue;
        };

        let mvp = viewproj * model;
        let resolved = material.and_then(|mr| mats.resolve(mr.id));
        let material_plan = LitMaterialPlan::from_resolved(resolved.as_ref(), entry.base_color);

        let terrain_receive_shadows = terrain_receive_shadows_enabled(shadow_policy);
        terrain_shadow_bound |= terrain_receive_shadows;
        let terrain_shadow_texture = if !terrain_receive_shadows {
            lit.white_texture
        } else {
            shadow_texture
        };
        let terrain_local_shadow_texture = if pass.is_gbuffer() || !terrain_receive_shadows {
            lit.white_texture
        } else {
            local_shadow_texture
        };
        let key = entity_key
            ^ 0x7e44_1000_0000_0000u64
            ^ if pass.is_gbuffer() {
                0x0000_0000_0b00_0000u64
            } else {
                0
            }
            ^ ((terrain_shadow_texture.get() as u64) << 32)
            ^ ((terrain_local_shadow_texture.get() as u64) << 16);
        let (pipeline, base_tex, normal_tex, roughness_tex, sampler, material_params) =
            if let Some(layers) = surface_layers {
                for path in [
                    layers.forest_base_texture.as_str(),
                    layers.sand_base_texture.as_str(),
                    layers.rock_base_texture.as_str(),
                ] {
                    this.request_material_texture_with_view_hints(
                        path,
                        MaterialTextureStreamingClass::StreamingCritical,
                        screen_coverage,
                        distance_m,
                        232,
                        0,
                        224,
                    );
                }
                let forest_tex = this.material_texture_or_default(
                    r,
                    Some(layers.forest_base_texture.as_str()),
                    lit.white_texture,
                );
                let sand_tex = this.material_texture_or_default(
                    r,
                    Some(layers.sand_base_texture.as_str()),
                    lit.white_texture,
                );
                let rock_tex = this.material_texture_or_default(
                    r,
                    Some(layers.rock_base_texture.as_str()),
                    lit.white_texture,
                );
                (
                    if pass.is_gbuffer() {
                        lit.gbuffer_terrain_pipeline
                    } else {
                        lit.terrain_pipeline
                    },
                    forest_tex,
                    sand_tex,
                    rock_tex,
                    lit.repeat_sampler,
                    [
                        layers.patch_scale,
                        layers.blend_softness,
                        material_plan.material_params[1],
                        material_plan.material_params[3],
                    ],
                )
            } else {
                this.request_material_set_with_view_hints(
                    material_plan.base_color_texture,
                    material_plan.normal_texture,
                    material_plan.roughness_texture,
                    screen_coverage,
                    distance_m,
                    0,
                    false,
                );
                let base_tex = this.material_texture_or_default(
                    r,
                    material_plan.base_color_texture,
                    lit.white_texture,
                );
                let normal_tex = this.material_texture_or_default(
                    r,
                    material_plan.normal_texture,
                    lit.flat_normal_texture,
                );
                let roughness_tex = this.material_texture_or_default(
                    r,
                    material_plan.roughness_texture,
                    lit.white_texture,
                );
                let sampler = if material_plan.has_textures() {
                    lit.repeat_sampler
                } else {
                    lit.clamp_sampler
                };
                let pipeline = if pass.is_gbuffer() {
                    if material_plan.double_sided {
                        lit.gbuffer_double_sided_pipeline
                    } else {
                        lit.gbuffer_pipeline
                    }
                } else if material_plan.double_sided {
                    lit.double_sided_pipeline
                } else {
                    lit.pipeline
                };
                (
                    pipeline,
                    base_tex,
                    normal_tex,
                    roughness_tex,
                    sampler,
                    material_plan.material_params,
                )
            };

        let per = this.ensure_per_draw_ubo_with_binding(
            r,
            lit,
            key,
            base_tex,
            normal_tex,
            roughness_tex,
            terrain_shadow_texture,
            terrain_local_shadow_texture,
            sampler,
        )?;

        super::super::passes_ubo::write_lit_ubo_ex(
            r,
            per.ubo,
            mvp,
            model,
            material_plan.base_color,
            material_plan.emissive_radiance,
            material_plan.alpha_cutoff,
            material_plan.uv_transform,
            material_params,
            lights,
        )?;

        stream.push(IndexedDrawPacket {
            pipeline,
            bind_group: per.bg,
            vertex: BufferSlice::new(gpu.vb, 0),
            index: BufferSlice::new(gpu.ib, 0),
            index_format: IndexFormat::U32,
            args: DrawIndexedArgs::new(gpu.index_count),
        });
        terrain_submitted = terrain_submitted.saturating_add(1);
        this.diagnostics
            .overlay_metrics
            .record_indexed_triangles(gpu.index_count);
    }
    if runtime && route_diagnostics_due(this.frame.frame_index) {
        newengine_ulog_api::ulog::debug!(
            "terrain.draw_list: pass='{}' candidates={} planned={} submitted={} gpu_missing={} budget={} shadow_texture_bound={} policy='draw all resident demo ring; no terrain pop/degrade budget clamp'",
            pass.label(),
            terrain_candidate_count,
            terrain_planned_count,
            terrain_submitted,
            terrain_gpu_missing,
            terrain_forward_budget,
            terrain_shadow_bound,
        );
    }
    stream.emit_sorted(r)?;

    Ok(())
}

fn draw_procedural_terrain_wireframe(
    this: &mut RuntimeRenderController,
    r: &mut dyn newengine_core::render::RenderApi,
    scene: &newengine_scene::Scene,
    viewproj: Mat4,
) -> newengine_core::EngineResult<()> {
    use newengine_core::render::{BufferSlice, DrawArgs};

    const MAX_TERRAIN_WIRE_VERTICES: usize = 160_000;
    let world = scene.world();
    let mut bytes = Vec::new();
    let mut vertex_count = 0usize;

    'terrain: for (_entity, terrain, global) in world.query2::<ProceduralTerrain, GlobalTransform>()
    {
        let heightfield = terrain.heightfield.as_ref();
        let vx = heightfield.vertex_count_x();
        let vz = heightfield.vertex_count_z();
        if vx < 2 || vz < 2 {
            continue;
        }
        let edge_count = (vx - 1) * vz + (vz - 1) * vx;
        let stride = ((edge_count * 2).div_ceil(MAX_TERRAIN_WIRE_VERTICES)).max(1);
        let color = [
            terrain.base_color[0].max(0.25),
            terrain.base_color[1].max(0.25),
            terrain.base_color[2].max(0.25),
            0.95,
        ];
        let mut edge_index = 0usize;
        for z in 0..vz {
            for x in 0..vx - 1 {
                if edge_index.is_multiple_of(stride) {
                    if vertex_count + 2 > MAX_TERRAIN_WIRE_VERTICES {
                        break 'terrain;
                    }
                    push_terrain_wire_edge(
                        &mut bytes,
                        &mut vertex_count,
                        viewproj,
                        global.0,
                        heightfield.local_position_at_grid(x, z),
                        heightfield.local_position_at_grid(x + 1, z),
                        color,
                    );
                }
                edge_index += 1;
            }
        }
        for x in 0..vx {
            for z in 0..vz - 1 {
                if edge_index.is_multiple_of(stride) {
                    if vertex_count + 2 > MAX_TERRAIN_WIRE_VERTICES {
                        break 'terrain;
                    }
                    push_terrain_wire_edge(
                        &mut bytes,
                        &mut vertex_count,
                        viewproj,
                        global.0,
                        heightfield.local_position_at_grid(x, z),
                        heightfield.local_position_at_grid(x, z + 1),
                        color,
                    );
                }
                edge_index += 1;
            }
        }
    }

    if vertex_count < 2 {
        return Ok(());
    }
    let gpu = crate::render_controller::gpu::ensure_debug_line_pipeline(
        &mut this.gpu.meshes.collision_lines,
        r,
        vertex_count as u32,
    )?;
    r.write_buffer(gpu.vb, 0, &bytes)?;
    r.set_pipeline(gpu.pipeline)?;
    r.set_bind_group(0, gpu.bg)?;
    r.set_vertex_buffer(0, BufferSlice::new(gpu.vb, 0))?;
    r.draw(DrawArgs::new(vertex_count as u32))?;
    Ok(())
}

fn push_terrain_wire_edge(
    bytes: &mut Vec<u8>,
    vertex_count: &mut usize,
    viewproj: Mat4,
    model: Mat4,
    a: Vec3,
    b: Vec3,
    color: [f32; 4],
) {
    use newengine_math::Vec4;
    for local in [a, b] {
        let position = model.transform_point3(local);
        let clip = viewproj * Vec4::new(position.x, position.y, position.z, 1.0);
        for value in [
            clip.x, clip.y, clip.z, clip.w, color[0], color[1], color[2], color[3],
        ] {
            bytes.extend_from_slice(&value.to_ne_bytes());
        }
        *vertex_count += 1;
    }
}

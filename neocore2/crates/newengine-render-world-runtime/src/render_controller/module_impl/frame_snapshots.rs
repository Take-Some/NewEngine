#![forbid(unsafe_op_in_unsafe_fn)]

use newengine_bounds::Bounds;
use newengine_core::render::{BindGroupId, Extent2D, PipelineId, TextureId};
use newengine_materials::MaterialRef;
use newengine_math::collections::FxHashMap;
use newengine_math::{Mat4, Vec3};
use newengine_model_domain_api::{FoliageInstanceRuntime, MeshRenderOptions};
use newengine_primitives::Primitive;
use newengine_render_feature_api::BoundsSnap;
use newengine_scene::Scene;
use newengine_transform::{GlobalTransform, TransformPropagationChanges};
use std::sync::Arc;

use newengine_gameplay_world_runtime::gameplay::{
    display_shadow_caster_visible_in_mode, display_visible_in_mode, player_render_model_matrix,
    DisplayVisibility, EnvironmentDomeRenderState, PlayerRenderPose, PlayerSkinBinding,
    PlayerVisualKind, PlayerVisualPart, WorldItemPresentation, WorldItemVisualPart,
};

use crate::render_controller::gpu::{PlayerSkinGpu, PrimitiveGpu};

use super::{passes::mesh_visibility::transform_sphere, scene, RuntimeRenderController};

/// Immutable primitive input captured once for all render passes in a frame.
///
/// The snapshot owns only CPU-side domain values. It keeps ECS access and gameplay
/// presentation rules outside backend recording while giving every pass a coherent
/// view of the project scene.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct PrimitiveSceneMembershipStamp {
    primitive: u64,
    global_transform: u64,
    player_skin: u64,
    visibility: u64,
    material: u64,
    render_options: u64,
    foliage: u64,
    environment_dome: u64,
    bounds: u64,
    player_visual: u64,
    player_render_pose: u64,
    world_item_visual: u64,
    world_item_presentation: u64,
}

impl PrimitiveSceneMembershipStamp {
    #[inline]
    fn capture(world: &newengine_ecs::World) -> Self {
        Self {
            primitive: world.component_membership_revision::<Primitive>(),
            global_transform: world.component_membership_revision::<GlobalTransform>(),
            player_skin: world.component_membership_revision::<PlayerSkinBinding>(),
            visibility: world.component_membership_revision::<DisplayVisibility>(),
            material: world.component_membership_revision::<MaterialRef>(),
            render_options: world.component_membership_revision::<MeshRenderOptions>(),
            foliage: world.component_membership_revision::<FoliageInstanceRuntime>(),
            environment_dome: world.component_membership_revision::<EnvironmentDomeRenderState>(),
            bounds: world.component_membership_revision::<Bounds>(),
            player_visual: world.component_membership_revision::<PlayerVisualPart>(),
            player_render_pose: world.component_membership_revision::<PlayerRenderPose>(),
            world_item_visual: world.component_membership_revision::<WorldItemVisualPart>(),
            world_item_presentation: world.component_membership_revision::<WorldItemPresentation>(),
        }
    }
}

/// Persistent CPU-side render scene snapshot. Static membership/metadata survives across frames;
/// high-frequency transform/presentation changes are patched into the cached entries.
#[derive(Clone)]
pub(in crate::render_controller) struct PrimitiveSceneSnapshot {
    frame_index: u64,
    observed_tick: u64,
    transform_generation: u64,
    scene_key: usize,
    runtime: bool,
    membership: PrimitiveSceneMembershipStamp,
    entity_index: FxHashMap<u64, usize>,
    player_owner_index: FxHashMap<u64, Vec<usize>>,
    pub(super) queried_count: usize,
    pub(super) entries: Box<[PrimitiveSceneEntry]>,
}

#[derive(Clone)]
pub(super) struct PrimitiveSceneEntry {
    pub(super) entity: newengine_ecs::EntityId,
    pub(super) entity_key: u64,
    render_owner: Option<newengine_ecs::EntityId>,
    pub(super) primitive: Primitive,
    pub(super) render_model: Mat4,
    pub(super) material_ref: Option<MaterialRef>,
    pub(super) render_options: MeshRenderOptions,
    pub(super) foliage_runtime: Option<FoliageInstanceRuntime>,
    pub(super) environment_dome: Option<EnvironmentDomeRenderState>,
    pub(super) local_bounds: Option<(Vec3, f32)>,
    pub(super) world_bounds: Option<(Vec3, f32)>,
    pub(super) authored_pbr_required: bool,
}

#[inline]
fn refresh_primitive_world_bounds(entry: &mut PrimitiveSceneEntry) {
    entry.world_bounds = entry
        .local_bounds
        .map(|(center, radius)| transform_sphere(entry.render_model, center, radius));
}

impl PrimitiveSceneSnapshot {
    fn capture(frame_index: u64, scene: &Scene, runtime: bool) -> Self {
        let world = scene.world();
        let mut queried_count = 0usize;
        let mut entries = Vec::new();

        for (id, primitive, transform) in world.query2::<Primitive, GlobalTransform>() {
            queried_count = queried_count.saturating_add(1);
            if world.get::<PlayerSkinBinding>(id).is_some()
                || !display_visible_in_mode(world, id, runtime)
            {
                continue;
            }

            let render_model = player_render_model_matrix(world, id, transform.0);
            let render_options = world
                .get::<MeshRenderOptions>(id)
                .cloned()
                .unwrap_or_else(MeshRenderOptions::world_opaque);
            let authored_pbr_required = world
                .get::<PlayerVisualPart>(id)
                .is_some_and(|part| part.kind == PlayerVisualKind::EquippedWeapon)
                || world
                    .get::<WorldItemVisualPart>(id)
                    .and_then(|part| world.get::<WorldItemPresentation>(part.owner))
                    .and_then(|presentation| presentation.model_ref.as_deref())
                    .is_some_and(|model_ref| !model_ref.trim().is_empty());
            let local_bounds = world
                .get::<Bounds>(id)
                .map(|bounds| (bounds.local_sphere.center, bounds.local_sphere.radius));
            let world_bounds = local_bounds
                .map(|(center, radius)| transform_sphere(render_model, center, radius));

            let render_owner = world.get::<PlayerVisualPart>(id).map(|part| part.owner);
            entries.push(PrimitiveSceneEntry {
                entity: id,
                entity_key: id.stable_u64(),
                render_owner,
                primitive: *primitive,
                render_model,
                material_ref: world.get::<MaterialRef>(id).copied(),
                render_options,
                foliage_runtime: world.get::<FoliageInstanceRuntime>(id).copied(),
                environment_dome: world.get::<EnvironmentDomeRenderState>(id).cloned(),
                local_bounds,
                world_bounds,
                authored_pbr_required,
            });
        }

        let mut entity_index = FxHashMap::default();
        let mut player_owner_index: FxHashMap<u64, Vec<usize>> = FxHashMap::default();
        for (index, entry) in entries.iter().enumerate() {
            entity_index.insert(entry.entity_key, index);
            if let Some(owner) = entry.render_owner {
                player_owner_index
                    .entry(owner.stable_u64())
                    .or_default()
                    .push(index);
            }
        }
        let transform_generation = world
            .resource::<TransformPropagationChanges>()
            .map(|changes| changes.generation)
            .unwrap_or(0);
        Self {
            frame_index,
            observed_tick: world.tick(),
            transform_generation,
            scene_key: scene as *const Scene as usize,
            runtime,
            membership: PrimitiveSceneMembershipStamp::capture(world),
            entity_index,
            player_owner_index,
            queried_count,
            entries: entries.into_boxed_slice(),
        }
    }

    #[inline]
    fn matches_scene(&self, scene: &Scene, runtime: bool) -> bool {
        self.scene_key == scene as *const Scene as usize && self.runtime == runtime
    }

    fn refresh_for_frame(&mut self, frame_index: u64, scene: &Scene) -> bool {
        if self.frame_index == frame_index {
            return true;
        }
        let world = scene.world();
        if world.tick() < self.observed_tick
            || self.membership != PrimitiveSceneMembershipStamp::capture(world)
        {
            return false;
        }
        let since_tick = self.observed_tick;

        // These values can change whether an entity belongs in the snapshot or alter ownership
        // semantics. They are rare; rebuild conservatively instead of maintaining complex deltas.
        if world.any_changed_since::<DisplayVisibility>(since_tick)
            || world.any_changed_since::<PlayerSkinBinding>(since_tick)
            || world.any_changed_since::<PlayerVisualPart>(since_tick)
            || world.any_changed_since::<WorldItemVisualPart>(since_tick)
            || world.any_changed_since::<WorldItemPresentation>(since_tick)
        {
            return false;
        }

        if world.any_changed_since::<Primitive>(since_tick) {
            for (entity, primitive) in world.query_changed::<Primitive>(since_tick) {
                if let Some(index) = self.entity_index.get(&entity.stable_u64()).copied() {
                    self.entries[index].primitive = *primitive;
                }
            }
        }
        if world.any_changed_since::<MaterialRef>(since_tick) {
            for (entity, material) in world.query_changed::<MaterialRef>(since_tick) {
                if let Some(index) = self.entity_index.get(&entity.stable_u64()).copied() {
                    self.entries[index].material_ref = Some(*material);
                }
            }
        }
        if world.any_changed_since::<MeshRenderOptions>(since_tick) {
            for (entity, options) in world.query_changed::<MeshRenderOptions>(since_tick) {
                if let Some(index) = self.entity_index.get(&entity.stable_u64()).copied() {
                    self.entries[index].render_options = options.clone();
                }
            }
        }
        if world.any_changed_since::<FoliageInstanceRuntime>(since_tick) {
            for (entity, foliage) in world.query_changed::<FoliageInstanceRuntime>(since_tick) {
                if let Some(index) = self.entity_index.get(&entity.stable_u64()).copied() {
                    self.entries[index].foliage_runtime = Some(*foliage);
                }
            }
        }
        if world.any_changed_since::<EnvironmentDomeRenderState>(since_tick) {
            for (entity, dome) in world.query_changed::<EnvironmentDomeRenderState>(since_tick) {
                if let Some(index) = self.entity_index.get(&entity.stable_u64()).copied() {
                    self.entries[index].environment_dome = Some(dome.clone());
                }
            }
        }
        if world.any_changed_since::<Bounds>(since_tick) {
            for (entity, bounds) in world.query_changed::<Bounds>(since_tick) {
                if let Some(index) = self.entity_index.get(&entity.stable_u64()).copied() {
                    let entry = &mut self.entries[index];
                    entry.local_bounds =
                        Some((bounds.local_sphere.center, bounds.local_sphere.radius));
                    refresh_primitive_world_bounds(entry);
                }
            }
        }

        let global_changed = world.any_changed_since::<GlobalTransform>(since_tick);
        let transform_journal = world.resource::<TransformPropagationChanges>();
        let current_generation = transform_journal
            .map(|changes| changes.generation)
            .unwrap_or(self.transform_generation);
        if current_generation == self.transform_generation {
            // A direct GlobalTransform write outside propagation cannot be represented by the
            // compact journal. Fall back to a complete recapture rather than serving stale data.
            if global_changed {
                return false;
            }
        } else if current_generation == self.transform_generation.saturating_add(1) {
            let Some(changes) = transform_journal else {
                return false;
            };
            for entity in changes.entities.iter().copied() {
                let Some(index) = self.entity_index.get(&entity.stable_u64()).copied() else {
                    continue;
                };
                let Some(global) = world.get::<GlobalTransform>(entity) else {
                    return false;
                };
                let entry = &mut self.entries[index];
                entry.render_model = player_render_model_matrix(world, entity, global.0);
                refresh_primitive_world_bounds(entry);
            }
        } else if global_changed {
            // More than one propagation happened while this scene was not rendered; the latest
            // journal is not cumulative, so a rebuild is the only safe recovery.
            return false;
        }
        self.transform_generation = current_generation;

        if world.any_changed_since::<PlayerRenderPose>(since_tick) {
            for (owner, _) in world.query_changed::<PlayerRenderPose>(since_tick) {
                let Some(indices) = self.player_owner_index.get(&owner.stable_u64()) else {
                    continue;
                };
                for &index in indices {
                    let entity = self.entries[index].entity;
                    let Some(global) = world.get::<GlobalTransform>(entity) else {
                        return false;
                    };
                    let entry = &mut self.entries[index];
                    entry.render_model = player_render_model_matrix(world, entity, global.0);
                    refresh_primitive_world_bounds(entry);
                }
            }
        }

        self.frame_index = frame_index;
        self.observed_tick = world.tick();
        true
    }
}

#[cfg(test)]
mod primitive_scene_snapshot_tests {
    use super::*;
    use newengine_transform::{propagate_transforms, Transform};

    fn scene_with_primitive() -> (Scene, newengine_ecs::EntityId) {
        let mut scene = Scene::new();
        let entity = scene.world_mut().spawn();
        assert!(scene.world_mut().insert(entity, Primitive::default()));
        assert!(scene
            .world_mut()
            .insert(entity, GlobalTransform::default()));
        (scene, entity)
    }

    #[test]
    fn persistent_snapshot_reuses_static_scene_across_frames() {
        let (mut scene, _) = scene_with_primitive();
        let mut snapshot = PrimitiveSceneSnapshot::capture(1, &scene, true);
        let first_entries_ptr = snapshot.entries.as_ptr();

        scene.world_mut().advance_tick();
        assert!(snapshot.refresh_for_frame(2, &scene));
        assert_eq!(snapshot.entries.as_ptr(), first_entries_ptr);
        assert_eq!(snapshot.frame_index, 2);
    }

    #[test]
    fn persistent_snapshot_patches_primitive_values_without_rebuild() {
        let (mut scene, entity) = scene_with_primitive();
        let mut snapshot = PrimitiveSceneSnapshot::capture(1, &scene, true);
        let first_entries_ptr = snapshot.entries.as_ptr();

        scene.world_mut().advance_tick();
        let mut primitive = *scene.world().get::<Primitive>(entity).unwrap();
        primitive.color = [0.2, 0.4, 0.6, 1.0];
        assert!(scene.world_mut().insert(entity, primitive));

        assert!(snapshot.refresh_for_frame(2, &scene));
        assert_eq!(snapshot.entries.as_ptr(), first_entries_ptr);
        assert_eq!(snapshot.entries[0].primitive.color, [0.2, 0.4, 0.6, 1.0]);
    }

    #[test]
    fn persistent_snapshot_patches_transform_journal_delta() {
        let mut scene = Scene::new();
        let entity = scene.world_mut().spawn();
        assert!(scene.world_mut().insert(entity, Primitive::default()));
        assert!(scene.world_mut().insert(entity, Transform::default()));
        assert!(scene.world_mut().insert(
            entity,
            Bounds::from_local_sphere(newengine_bounds::Sphere::new(Vec3::ZERO, 2.0)),
        ));
        propagate_transforms(scene.world_mut());
        let mut snapshot = PrimitiveSceneSnapshot::capture(1, &scene, true);

        scene.world_mut().advance_tick();
        scene.world_mut().get_mut::<Transform>(entity).unwrap().position = Vec3::new(7.0, 0.0, -2.0);
        propagate_transforms(scene.world_mut());

        assert!(snapshot.refresh_for_frame(2, &scene));
        let world_origin = snapshot.entries[0].render_model.transform_point3(Vec3::ZERO);
        assert!((world_origin - Vec3::new(7.0, 0.0, -2.0)).length() < 1.0e-6);
        let (bounds_center, bounds_radius) = snapshot.entries[0].world_bounds.unwrap();
        assert!((bounds_center - world_origin).length() < 1.0e-6);
        assert!((bounds_radius - 2.0).abs() < 1.0e-6);
    }

    #[test]
    fn persistent_snapshot_rejects_structural_membership_change() {
        let (mut scene, _) = scene_with_primitive();
        let mut snapshot = PrimitiveSceneSnapshot::capture(1, &scene, true);

        scene.world_mut().advance_tick();
        let added = scene.world_mut().spawn();
        assert!(scene.world_mut().insert(added, Primitive::default()));
        assert!(scene
            .world_mut()
            .insert(added, GlobalTransform::default()));

        assert!(!snapshot.refresh_for_frame(2, &scene));
    }
}

/// Frame-coherent admission set for skinned directional-shadow casters.
///
/// CSM cascades share entity/material/owner admission; only the light matrix and
/// projected-size decision vary per cascade. Capturing this once prevents four
/// complete ECS scans of the same character parts every frame.
pub(in crate::render_controller) struct SkinnedShadowSceneSnapshot {
    frame_index: u64,
    scene_key: usize,
    runtime: bool,
    pub(super) entries: Box<[SkinnedShadowSceneEntry]>,
}

pub(super) struct SkinnedShadowSceneEntry {
    pub(super) entity: newengine_ecs::EntityId,
    pub(super) owner: newengine_ecs::EntityId,
    pub(super) primitive: Primitive,
    pub(super) render_model: Mat4,
    pub(super) material_ref: Option<MaterialRef>,
    pub(super) proxy_center_ws: Vec3,
    pub(super) proxy_radius_ws: f32,
    pub(super) pose_generation: u64,
}

/// GPU/material state resolved once per frame for all skinned shadow views.
///
/// The renderer remains the owner of every native resource. This immutable plan stores only
/// frame-local handles and draw constants so directional cascades do not repeat ECS/material/GPU
/// cache resolution. Cascade-specific visibility and light matrices intentionally remain outside.
pub(in crate::render_controller) struct PreparedSkinnedShadowFramePlan {
    pub(super) frame_index: u64,
    pub(super) scene_key: usize,
    pub(super) runtime: bool,
    pub(super) entries: Box<[PreparedSkinnedShadowCaster]>,
}

#[derive(Clone, Copy)]
pub(super) struct PreparedSkinnedShadowCaster {
    pub(super) entity: newengine_ecs::EntityId,
    pub(super) primitive: Primitive,
    pub(super) render_model: Mat4,
    pub(super) proxy_center_ws: Vec3,
    pub(super) proxy_radius_ws: f32,
    pub(super) primitive_gpu: PrimitiveGpu,
    pub(super) skin_gpu: PlayerSkinGpu,
    pub(super) palette_bg: BindGroupId,
    pub(super) base_texture: TextureId,
    pub(super) pipeline: PipelineId,
    pub(super) alpha_cutoff: f32,
    pub(super) uv_transform: [f32; 4],
}

impl PreparedSkinnedShadowFramePlan {
    #[inline]
    pub(super) fn matches(&self, frame_index: u64, scene: &Scene, runtime: bool) -> bool {
        self.frame_index == frame_index
            && self.scene_key == scene as *const Scene as usize
            && self.runtime == runtime
    }
}

impl SkinnedShadowSceneSnapshot {
    fn capture(frame_index: u64, scene: &Scene, runtime: bool) -> Self {
        let world = scene.world();
        let mut entries = Vec::new();
        for (entity, primitive, global) in world.query2::<Primitive, GlobalTransform>() {
            let Some(skin) = world.get::<PlayerSkinBinding>(entity) else {
                continue;
            };
            if !display_shadow_caster_visible_in_mode(world, entity, runtime) {
                continue;
            }
            let render_model = player_render_model_matrix(world, entity, global.0);
            let owner_height = world
                .get::<newengine_gameplay_world_runtime::gameplay::PlayerModelBinding>(skin.owner)
                .map(|binding| binding.target_height.max(1.0))
                .unwrap_or(2.0);
            let proxy_center_ws = world
                .get::<GlobalTransform>(skin.owner)
                .map(|owner_global| {
                    owner_global
                        .0
                        .transform_point3(Vec3::new(0.0, owner_height * 0.5, 0.0))
                })
                .unwrap_or_else(|| render_model.transform_point3(Vec3::ZERO));
            let pose_generation = world
                .get::<newengine_gameplay_world_runtime::gameplay::PlayerModelBinding>(skin.owner)
                .map(|binding| binding.assignment_revision)
                .unwrap_or(0);
            entries.push(SkinnedShadowSceneEntry {
                entity,
                owner: skin.owner,
                primitive: *primitive,
                render_model,
                material_ref: world.get::<MaterialRef>(entity).copied(),
                proxy_center_ws,
                proxy_radius_ws: owner_height * 0.80 + 0.45,
                pose_generation,
            });
        }
        Self {
            frame_index,
            scene_key: scene as *const Scene as usize,
            runtime,
            entries: entries.into_boxed_slice(),
        }
    }

    #[inline]
    fn matches(&self, frame_index: u64, scene: &Scene, runtime: bool) -> bool {
        self.frame_index == frame_index
            && self.scene_key == scene as *const Scene as usize
            && self.runtime == runtime
    }
}

impl RuntimeRenderController {
    fn synchronize_gpu_scene_tables(&mut self, snapshot: &PrimitiveSceneSnapshot) {
        if !newengine_runtime_env::var_bool("NEWENGINE_GPU_SCENE_TABLES_ENABLE", false) {
            return;
        }
        let materials_lock = self.bridges.scene.materials();
        let materials = materials_lock.read();
        let gpu = &mut self.gpu;
        gpu.tables.synchronize_cpu(
            self.frame.frame_index,
            snapshot,
            &gpu.geometry,
            &*materials,
        );
        if newengine_ulog_api::ulog::trace_enabled()
            && (self.frame.frame_index <= 3 || self.frame.frame_index.is_multiple_of(300))
        {
            let stats = gpu.tables.stats();
            newengine_ulog_api::ulog::trace!(
                "render gpu scene tables: frame={} geometry={}/{} materials={}/{} objects={}/{} material_revision={} gate='NEWENGINE_GPU_SCENE_TABLES_ENABLE'",
                self.frame.frame_index,
                stats.geometry_resident,
                stats.geometry_slots,
                stats.material_resident,
                stats.material_slots,
                stats.object_resident,
                stats.object_slots,
                stats.material_revision,
            );
        }
    }

    /// Returns the frame-coherent primitive snapshot and whether it was already captured.
    pub(super) fn primitive_scene_snapshot(
        &mut self,
        scene: &Scene,
        runtime: bool,
    ) -> (Arc<PrimitiveSceneSnapshot>, bool) {
        let frame_index = self.frame.frame_index;
        let reused = if let Some(snapshot) = self.frame.primitive_scene_snapshot.as_mut() {
            if snapshot.matches_scene(scene, runtime) {
                let reusable = Arc::make_mut(snapshot).refresh_for_frame(frame_index, scene);
                reusable.then(|| Arc::clone(snapshot))
            } else {
                None
            }
        } else {
            None
        };
        if let Some(snapshot) = reused {
            self.synchronize_gpu_scene_tables(snapshot.as_ref());
            return (snapshot, true);
        }

        let snapshot = Arc::new(PrimitiveSceneSnapshot::capture(frame_index, scene, runtime));
        self.frame.primitive_scene_snapshot = Some(Arc::clone(&snapshot));
        self.synchronize_gpu_scene_tables(snapshot.as_ref());
        (snapshot, false)
    }
    /// Returns skinned shadow admission captured once for all CSM cascades.
    pub(super) fn skinned_shadow_scene_snapshot(
        &mut self,
        scene: &Scene,
        runtime: bool,
    ) -> (Arc<SkinnedShadowSceneSnapshot>, bool) {
        let frame_index = self.frame.frame_index;
        if let Some(snapshot) = self.frame.skinned_shadow_scene_snapshot.as_ref() {
            if snapshot.matches(frame_index, scene, runtime) {
                return (Arc::clone(snapshot), true);
            }
        }
        let snapshot = Arc::new(SkinnedShadowSceneSnapshot::capture(
            frame_index,
            scene,
            runtime,
        ));
        self.frame.skinned_shadow_scene_snapshot = Some(Arc::clone(&snapshot));
        (snapshot, false)
    }
}

/// CPU-side scene render snapshot captured before RenderPrep/submit.
///
/// This is the first structural boundary for moving provider-safe extraction out
/// of `render.controller`. It intentionally contains DTO-like values, not
/// `RenderApi`, backend handles or mutable world references. Heavy consumers can
/// later receive this through `engine.threading` RenderPrep batches and return frame
/// packets for render-thread recording.
#[derive(Clone, Copy, Debug)]
pub(super) struct SceneRenderSnapshot {
    pub frame_index: u64,
    pub bounds: BoundsSnap,
    pub camera_position: Vec3,
    pub camera_forward: Vec3,
    pub viewport_extent: Extent2D,
    pub surface_extent: Extent2D,
    pub ui_present: bool,
    pub plugin_snapshot_present: bool,
}

impl SceneRenderSnapshot {
    #[allow(clippy::too_many_arguments)]
    pub(super) fn capture(
        frame_index: u64,
        scene: &Scene,
        _viewproj: Mat4,
        camera_position: Vec3,
        camera_forward: Vec3,
        viewport_extent: Extent2D,
        surface_extent: Extent2D,
        ui_present: bool,
        plugin_snapshot_present: bool,
    ) -> Self {
        Self {
            frame_index,
            bounds: scene::scene_bounds(scene).unwrap_or_else(scene::default_bounds),
            camera_position,
            camera_forward,
            viewport_extent,
            surface_extent,
            ui_present,
            plugin_snapshot_present,
        }
    }

    pub(super) fn diagnostic_detail(&self) -> String {
        format!(
            "SceneRenderSnapshot frame={} bounds_radius={:.3} viewport={}x{} surface={}x{} ui_present={} plugin_snapshot={}",
            self.frame_index,
            self.bounds.radius,
            self.viewport_extent.width,
            self.viewport_extent.height,
            self.surface_extent.width,
            self.surface_extent.height,
            self.ui_present,
            self.plugin_snapshot_present,
        )
    }
}

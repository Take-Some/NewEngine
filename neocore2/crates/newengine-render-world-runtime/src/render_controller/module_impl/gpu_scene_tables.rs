#![forbid(unsafe_op_in_unsafe_fn)]

use newengine_materials::api::MaterialRegistryApi;
use newengine_materials::{
    MaterialDomain, MaterialFlags, MaterialId, MaterialResolved, ShadingModel,
};
use newengine_math::collections::{FxHashMap, FxHashSet};
use newengine_math::Mat4;
use newengine_model_domain_api::{
    MeshCullPolicy, MeshDepthPolicy, MeshRenderRole, MeshShadowPolicy, MeshSortPolicy,
    MeshTransformPolicy,
};

use super::frame_snapshots::PrimitiveSceneSnapshot;
use crate::render_controller::gpu::{GeometryArena, GeometryHandle};

pub(in crate::render_controller) const INVALID_GPU_SCENE_SLOT: u32 = u32::MAX;
pub(in crate::render_controller) const INVALID_BINDLESS_INDEX: u32 = u32::MAX;

const MATERIAL_FLAG_ACTIVE: u32 = 1 << 31;
const MATERIAL_FLAG_TEXTURE_BINDINGS: u32 = 1 << 30;
const OBJECT_FLAG_ACTIVE: u32 = 1 << 0;
const OBJECT_FLAG_GEOMETRY_RESIDENT: u32 = 1 << 1;
const OBJECT_FLAG_MATERIAL_RESOLVED: u32 = 1 << 2;
const OBJECT_FLAG_AUTHORED_PBR: u32 = 1 << 3;
const OBJECT_FLAG_FOLIAGE: u32 = 1 << 4;
const OBJECT_FLAG_ENVIRONMENT_DOME: u32 = 1 << 5;
const OBJECT_FLAG_GBUFFER_INDIRECT_ELIGIBLE: u32 = 1 << 6;
const OBJECT_FLAG_SHADOW_INDIRECT_ELIGIBLE: u32 = 1 << 7;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub(in crate::render_controller) struct MaterialTableHandle {
    pub(in crate::render_controller) slot: u32,
    pub(in crate::render_controller) generation: u32,
}

impl MaterialTableHandle {
    #[inline]
    pub(in crate::render_controller) const fn invalid() -> Self {
        Self {
            slot: INVALID_GPU_SCENE_SLOT,
            generation: 0,
        }
    }

    #[inline]
    pub(in crate::render_controller) const fn is_valid(self) -> bool {
        self.slot != INVALID_GPU_SCENE_SLOT && self.generation != 0
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub(in crate::render_controller) struct ObjectTableHandle {
    pub(in crate::render_controller) slot: u32,
    pub(in crate::render_controller) generation: u32,
}

impl ObjectTableHandle {
    #[inline]
    pub(in crate::render_controller) const fn invalid() -> Self {
        Self {
            slot: INVALID_GPU_SCENE_SLOT,
            generation: 0,
        }
    }
}

/// GPU-facing geometry table row. Exactly 64 bytes / four vec4 lanes.
///
/// A page selects one common vertex/index arena pair. `draw[2]` stores the signed Vulkan
/// `vertexOffset` bit pattern. Slot generation is checked by object records before a future
/// GPU-driven draw stream is admitted.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default)]
pub(in crate::render_controller) struct GpuGeometryRecord {
    pub(in crate::render_controller) meta: [u32; 4],
    pub(in crate::render_controller) draw: [u32; 4],
    pub(in crate::render_controller) bounds: [f32; 4],
    pub(in crate::render_controller) reserved: [u32; 4],
}

impl GpuGeometryRecord {
    #[inline]
    fn invalid(generation: u32) -> Self {
        Self {
            meta: [generation, 0, INVALID_GPU_SCENE_SLOT, 0],
            ..Self::default()
        }
    }
}

/// GPU-facing material row. Exactly 96 bytes / six vec4 lanes.
///
/// `texture_indices` intentionally contains bindless table indices, not backend `TextureId`
/// handles. Until descriptor indexing is negotiated, every lane remains `u32::MAX` and the
/// legacy bind-group path stays authoritative.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default)]
pub(in crate::render_controller) struct GpuMaterialRecord {
    pub(in crate::render_controller) meta: [u32; 4],
    pub(in crate::render_controller) texture_indices: [u32; 4],
    pub(in crate::render_controller) base_color: [f32; 4],
    pub(in crate::render_controller) emissive_alpha: [f32; 4],
    pub(in crate::render_controller) uv_transform: [f32; 4],
    pub(in crate::render_controller) params: [f32; 4],
}

impl GpuMaterialRecord {
    #[inline]
    fn has_texture_bindings(material: &MaterialResolved) -> bool {
        let textures = &material.textures;
        textures.base_color_texture.is_some()
            || textures.normal_texture.is_some()
            || textures.metallic_texture.is_some()
            || textures.roughness_texture.is_some()
            || textures.occlusion_texture.is_some()
            || textures.emissive_texture.is_some()
    }

    #[inline]
    fn gbuffer_constants_compatible(&self) -> bool {
        let flags = self.meta[1];
        let unsupported_surface_flags = MaterialFlags::DOUBLE_SIDED.0
            | MaterialFlags::ALPHA_BLEND.0
            | MaterialFlags::ALPHA_TEST.0;
        let receives_shadows = (flags & MaterialFlags::RECEIVE_SHADOWS.0) != 0;
        (flags & MATERIAL_FLAG_ACTIVE) != 0
            && (flags & MATERIAL_FLAG_TEXTURE_BINDINGS) == 0
            && (flags & unsupported_surface_flags) == 0
            && receives_shadows
            && self.meta[2] == MaterialDomain::Surface as u32
            && self.meta[3] == ShadingModel::PbrMetallicRoughness as u32
    }
    #[inline]
    fn shadow_opaque_compatible(&self) -> bool {
        let flags = self.meta[1];
        let unsupported = MaterialFlags::DOUBLE_SIDED.0
            | MaterialFlags::ALPHA_BLEND.0
            | MaterialFlags::ALPHA_TEST.0;
        (flags & MATERIAL_FLAG_ACTIVE) != 0
            && (flags & MaterialFlags::CAST_SHADOWS.0) != 0
            && (flags & unsupported) == 0
            && self.meta[2] == MaterialDomain::Surface as u32
    }

    #[inline]
    fn invalid(generation: u32) -> Self {
        Self {
            meta: [generation, 0, 0, 0],
            texture_indices: [INVALID_BINDLESS_INDEX; 4],
            ..Self::default()
        }
    }

    fn from_resolved(generation: u32, material: &MaterialResolved) -> Self {
        let desc = material.desc.sanitized();
        let textures = material.textures.clone().sanitized();
        let emissive = desc.emissive_radiance();
        Self {
            meta: [
                generation,
                MATERIAL_FLAG_ACTIVE
                    | desc.flags.0
                    | if Self::has_texture_bindings(material) {
                        MATERIAL_FLAG_TEXTURE_BINDINGS
                    } else {
                        0
                    },
                desc.domain as u32,
                desc.shading_model as u32,
            ],
            texture_indices: [INVALID_BINDLESS_INDEX; 4],
            base_color: desc.base_color,
            emissive_alpha: [emissive[0], emissive[1], emissive[2], desc.alpha_cutoff],
            uv_transform: [
                textures.uv_scale[0],
                textures.uv_scale[1],
                textures.uv_offset[0],
                textures.uv_offset[1],
            ],
            params: [
                desc.metallic,
                desc.roughness,
                desc.normal_scale,
                desc.occlusion_strength,
            ],
        }
    }
}

/// GPU-facing object row. Exactly 128 bytes / eight vec4 lanes.
///
/// Identity and table-handle generations make stale references fail closed at table lookup,
/// while the render path itself remains fail-open because records without resident geometry are
/// never eligible for future indirect command generation.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default)]
pub(in crate::render_controller) struct GpuObjectRecord {
    pub(in crate::render_controller) identity: [u32; 4],
    pub(in crate::render_controller) handles: [u32; 4],
    pub(in crate::render_controller) model_cols: [[f32; 4]; 4],
    pub(in crate::render_controller) bounds: [f32; 4],
    pub(in crate::render_controller) fallback_color: [f32; 4],
}

impl GpuObjectRecord {
    #[inline]
    pub(in crate::render_controller) fn indirect_active(&self) -> bool {
        (self.identity[3] & OBJECT_FLAG_ACTIVE) != 0
            && (self.identity[3] & OBJECT_FLAG_GEOMETRY_RESIDENT) != 0
    }

    #[inline]
    pub(in crate::render_controller) fn indirect_cullable(&self) -> bool {
        self.indirect_active() && (self.identity[3] & OBJECT_FLAG_ENVIRONMENT_DOME) == 0
    }

    #[inline]
    pub(in crate::render_controller) fn gbuffer_indirect_eligible(&self) -> bool {
        self.indirect_active()
            && (self.identity[3] & OBJECT_FLAG_GBUFFER_INDIRECT_ELIGIBLE) != 0
    }
    #[inline]
    pub(in crate::render_controller) fn shadow_indirect_eligible(&self) -> bool {
        self.indirect_active()
            && (self.identity[3] & OBJECT_FLAG_SHADOW_INDIRECT_ELIGIBLE) != 0
    }

    #[inline]
    pub(in crate::render_controller) fn geometry_handle_lanes(&self) -> [u32; 2] {
        [self.handles[0], self.handles[1]]
    }
    #[inline]
    fn invalid(generation: u32) -> Self {
        Self {
            identity: [0, 0, generation, 0],
            handles: [INVALID_GPU_SCENE_SLOT, 0, INVALID_GPU_SCENE_SLOT, 0],
            ..Self::default()
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(in crate::render_controller) struct GpuSceneTableStats {
    pub(in crate::render_controller) geometry_slots: usize,
    pub(in crate::render_controller) geometry_resident: usize,
    pub(in crate::render_controller) material_slots: usize,
    pub(in crate::render_controller) material_resident: usize,
    pub(in crate::render_controller) object_slots: usize,
    pub(in crate::render_controller) object_resident: usize,
    pub(in crate::render_controller) material_revision: u64,
    pub(in crate::render_controller) last_object_sync_frame: u64,
}

#[derive(Clone, Copy, Debug)]
struct MaterialSlot {
    generation: u32,
    id: Option<MaterialId>,
    record: GpuMaterialRecord,
}

impl Default for MaterialSlot {
    fn default() -> Self {
        Self {
            generation: 1,
            id: None,
            record: GpuMaterialRecord::invalid(1),
        }
    }
}

#[derive(Debug, Default)]
struct StableMaterialTable {
    revision: u64,
    initialized: bool,
    slots: Vec<MaterialSlot>,
    by_id: FxHashMap<u64, MaterialTableHandle>,
    reusable: Vec<u32>,
}

impl StableMaterialTable {
    fn synchronize(&mut self, registry: &dyn MaterialRegistryApi) {
        let revision = registry.revision();
        if self.initialized && self.revision == revision {
            return;
        }

        let snapshot = registry.snapshot();
        let live_ids = snapshot
            .iter()
            .map(|item| item.id.raw())
            .collect::<FxHashSet<_>>();
        let stale = self
            .by_id
            .keys()
            .copied()
            .filter(|id| !live_ids.contains(id))
            .collect::<Vec<_>>();
        for raw in stale {
            self.retire_raw(raw);
        }

        for item in snapshot {
            let Some(resolved) = registry.resolve(item.id) else {
                self.retire_raw(item.id.raw());
                continue;
            };
            let handle = self
                .handle(item.id)
                .unwrap_or_else(|| self.allocate(item.id));
            let slot = &mut self.slots[handle.slot as usize];
            slot.record = GpuMaterialRecord::from_resolved(slot.generation, &resolved);
        }

        self.revision = revision;
        self.initialized = true;
    }

    #[inline]
    fn handle(&self, id: MaterialId) -> Option<MaterialTableHandle> {
        let handle = *self.by_id.get(&id.raw())?;
        let slot = self.slots.get(handle.slot as usize)?;
        (slot.generation == handle.generation && slot.id == Some(id)).then_some(handle)
    }

    fn allocate(&mut self, id: MaterialId) -> MaterialTableHandle {
        let slot_index = if let Some(slot) = self.reusable.pop() {
            slot
        } else {
            let slot = self.slots.len() as u32;
            self.slots.push(MaterialSlot::default());
            slot
        };
        let slot = &mut self.slots[slot_index as usize];
        slot.id = Some(id);
        let handle = MaterialTableHandle {
            slot: slot_index,
            generation: slot.generation,
        };
        self.by_id.insert(id.raw(), handle);
        handle
    }

    fn retire_raw(&mut self, raw: u64) {
        let Some(handle) = self.by_id.remove(&raw) else {
            return;
        };
        let Some(slot) = self.slots.get_mut(handle.slot as usize) else {
            return;
        };
        if slot.generation != handle.generation {
            return;
        }
        slot.id = None;
        slot.generation = next_generation(slot.generation);
        slot.record = GpuMaterialRecord::invalid(slot.generation);
        self.reusable.push(handle.slot);
    }

    fn records(&self) -> Vec<GpuMaterialRecord> {
        self.slots.iter().map(|slot| slot.record).collect()
    }
}

#[derive(Clone, Copy, Debug)]
struct ObjectSlot {
    generation: u32,
    entity_key: Option<u64>,
    last_seen_frame: u64,
    record: GpuObjectRecord,
}

impl Default for ObjectSlot {
    fn default() -> Self {
        Self {
            generation: 1,
            entity_key: None,
            last_seen_frame: 0,
            record: GpuObjectRecord::invalid(1),
        }
    }
}

#[derive(Debug, Default)]
struct StableObjectTable {
    slots: Vec<ObjectSlot>,
    by_entity: FxHashMap<u64, ObjectTableHandle>,
    reusable: Vec<u32>,
    last_sync_frame: u64,
}

impl StableObjectTable {
    fn synchronize(
        &mut self,
        frame: u64,
        snapshot: &PrimitiveSceneSnapshot,
        geometry: &GeometryArena,
        materials: &StableMaterialTable,
    ) {
        let mut live = FxHashSet::default();
        for entry in snapshot.entries.iter() {
            live.insert(entry.entity_key);
            let handle = self
                .handle(entry.entity_key)
                .unwrap_or_else(|| self.allocate(entry.entity_key));
            let geometry_handle = geometry
                .handle_for(entry.primitive.id)
                .unwrap_or_default();
            let material_handle = entry
                .material_ref
                .and_then(|material| materials.handle(material.id))
                .unwrap_or_else(MaterialTableHandle::invalid);

            let mut flags = OBJECT_FLAG_ACTIVE;
            if geometry_handle.generation != 0 {
                flags |= OBJECT_FLAG_GEOMETRY_RESIDENT;
            }
            if material_handle.is_valid() {
                flags |= OBJECT_FLAG_MATERIAL_RESOLVED;
            }
            if entry.authored_pbr_required {
                flags |= OBJECT_FLAG_AUTHORED_PBR;
            }
            if entry.foliage_runtime.is_some() {
                flags |= OBJECT_FLAG_FOLIAGE;
            }
            if entry.environment_dome.is_some() {
                flags |= OBJECT_FLAG_ENVIRONMENT_DOME;
            }
            let material_constants_compatible = material_handle.is_valid()
                && materials
                    .slots
                    .get(material_handle.slot as usize)
                    .is_some_and(|slot| {
                        slot.generation == material_handle.generation
                            && slot.record.gbuffer_constants_compatible()
                    });
            let gbuffer_indirect_eligible = geometry_handle.generation != 0
                && material_constants_compatible
                && !entry.authored_pbr_required
                && entry.foliage_runtime.is_none()
                && entry.environment_dome.is_none()
                && entry.render_options.role == MeshRenderRole::WorldOpaque
                && entry.render_options.transform_policy == MeshTransformPolicy::World
                && entry.render_options.depth_policy == MeshDepthPolicy::ReadWrite
                && entry.render_options.cull_policy == MeshCullPolicy::BackFace
                && entry.render_options.sort_policy == MeshSortPolicy::Opaque;
            if gbuffer_indirect_eligible {
                flags |= OBJECT_FLAG_GBUFFER_INDIRECT_ELIGIBLE;
            }
            let material_shadow_compatible = material_handle.is_valid()
                && materials
                    .slots
                    .get(material_handle.slot as usize)
                    .is_some_and(|slot| {
                        slot.generation == material_handle.generation
                            && slot.record.shadow_opaque_compatible()
                    });
            let shadow_indirect_eligible = geometry_handle.generation != 0
                && material_shadow_compatible
                && entry.foliage_runtime.is_none()
                && entry.environment_dome.is_none()
                && entry.render_options.role == MeshRenderRole::WorldOpaque
                && entry.render_options.transform_policy == MeshTransformPolicy::World
                && entry.render_options.cull_policy == MeshCullPolicy::BackFace
                && matches!(
                    entry.render_options.shadow_policy,
                    MeshShadowPolicy::CastOnly
                        | MeshShadowPolicy::CastAndReceive
                        | MeshShadowPolicy::ProfileControlled
                );
            if shadow_indirect_eligible {
                flags |= OBJECT_FLAG_SHADOW_INDIRECT_ELIGIBLE;
            }

            let (bounds_center, bounds_radius) = entry.world_bounds.unwrap_or_else(|| {
                let center = entry.render_model.transform_point3(newengine_math::Vec3::ZERO);
                (center, 0.001)
            });
            let slot = &mut self.slots[handle.slot as usize];
            slot.last_seen_frame = frame;
            slot.record = GpuObjectRecord {
                identity: [
                    entry.entity_key as u32,
                    (entry.entity_key >> 32) as u32,
                    slot.generation,
                    flags,
                ],
                handles: [
                    geometry_handle.slot,
                    geometry_handle.generation,
                    material_handle.slot,
                    material_handle.generation,
                ],
                model_cols: mat4_cols(entry.render_model),
                bounds: [
                    bounds_center.x,
                    bounds_center.y,
                    bounds_center.z,
                    bounds_radius.max(0.001),
                ],
                fallback_color: entry.primitive.color,
            };
        }

        let stale = self
            .by_entity
            .keys()
            .copied()
            .filter(|entity| !live.contains(entity))
            .collect::<Vec<_>>();
        for entity in stale {
            self.retire(entity);
        }
        self.last_sync_frame = frame;
    }

    #[inline]
    fn handle(&self, entity_key: u64) -> Option<ObjectTableHandle> {
        let handle = *self.by_entity.get(&entity_key)?;
        let slot = self.slots.get(handle.slot as usize)?;
        (slot.generation == handle.generation && slot.entity_key == Some(entity_key))
            .then_some(handle)
    }

    fn allocate(&mut self, entity_key: u64) -> ObjectTableHandle {
        let slot_index = if let Some(slot) = self.reusable.pop() {
            slot
        } else {
            let slot = self.slots.len() as u32;
            self.slots.push(ObjectSlot::default());
            slot
        };
        let slot = &mut self.slots[slot_index as usize];
        slot.entity_key = Some(entity_key);
        let handle = ObjectTableHandle {
            slot: slot_index,
            generation: slot.generation,
        };
        self.by_entity.insert(entity_key, handle);
        handle
    }

    fn retire(&mut self, entity_key: u64) {
        let Some(handle) = self.by_entity.remove(&entity_key) else {
            return;
        };
        let Some(slot) = self.slots.get_mut(handle.slot as usize) else {
            return;
        };
        if slot.generation != handle.generation {
            return;
        }
        slot.entity_key = None;
        slot.last_seen_frame = 0;
        slot.generation = next_generation(slot.generation);
        slot.record = GpuObjectRecord::invalid(slot.generation);
        self.reusable.push(handle.slot);
    }

    fn records(&self) -> Vec<GpuObjectRecord> {
        self.slots.iter().map(|slot| slot.record).collect()
    }
}

#[derive(Debug, Default)]
pub(in crate::render_controller) struct GpuSceneTables {
    materials: StableMaterialTable,
    objects: StableObjectTable,
    geometry_records: Vec<GpuGeometryRecord>,
    material_records: Vec<GpuMaterialRecord>,
    object_records: Vec<GpuObjectRecord>,
}

impl GpuSceneTables {
    pub(in crate::render_controller) fn synchronize_cpu(
        &mut self,
        frame: u64,
        snapshot: &PrimitiveSceneSnapshot,
        geometry: &GeometryArena,
        material_registry: &dyn MaterialRegistryApi,
    ) {
        self.materials.synchronize(material_registry);
        self.geometry_records = geometry
            .table_snapshot()
            .into_iter()
            .map(
                |(
                    generation,
                    resident,
                    page,
                    vertex_stride,
                    first_index,
                    vertex_offset,
                    vertex_count,
                    index_count,
                    bounds_center,
                    bounds_radius,
                )| {
                    if !resident {
                        return GpuGeometryRecord::invalid(generation);
                    }
                    GpuGeometryRecord {
                        meta: [generation, 1, page, vertex_stride],
                        draw: [
                            first_index,
                            index_count,
                            vertex_offset as u32,
                            vertex_count,
                        ],
                        bounds: [
                            bounds_center.x,
                            bounds_center.y,
                            bounds_center.z,
                            bounds_radius.max(0.001),
                        ],
                        reserved: [0; 4],
                    }
                },
            )
            .collect();
        self.objects
            .synchronize(frame, snapshot, geometry, &self.materials);

        self.material_records.clear();
        self.material_records
            .extend(self.materials.slots.iter().map(|slot| slot.record));
        self.object_records.clear();
        self.object_records
            .extend(self.objects.slots.iter().map(|slot| slot.record));
    }

    #[inline]
    pub(in crate::render_controller) fn object_handle(
        &self,
        entity_key: u64,
    ) -> Option<ObjectTableHandle> {
        self.objects.handle(entity_key)
    }

    #[inline]
    pub(in crate::render_controller) fn material_handle(
        &self,
        id: MaterialId,
    ) -> Option<MaterialTableHandle> {
        self.materials.handle(id)
    }

    #[inline]
    pub(in crate::render_controller) fn geometry_records(&self) -> &[GpuGeometryRecord] {
        &self.geometry_records
    }

    #[inline]
    pub(in crate::render_controller) fn material_records(&self) -> &[GpuMaterialRecord] {
        &self.material_records
    }

    #[inline]
    pub(in crate::render_controller) fn object_records(&self) -> &[GpuObjectRecord] {
        &self.object_records
    }

    pub(in crate::render_controller) fn stats(&self) -> GpuSceneTableStats {
        GpuSceneTableStats {
            geometry_slots: self.geometry_records.len(),
            geometry_resident: self
                .geometry_records
                .iter()
                .filter(|record| record.meta[1] != 0)
                .count(),
            material_slots: self.materials.slots.len(),
            material_resident: self.materials.by_id.len(),
            object_slots: self.objects.slots.len(),
            object_resident: self.objects.by_entity.len(),
            material_revision: self.materials.revision,
            last_object_sync_frame: self.objects.last_sync_frame,
        }
    }
}
#[inline]
fn mat4_cols(matrix: Mat4) -> [[f32; 4]; 4] {
    let raw = matrix.to_cols_array();
    [
        [raw[0], raw[1], raw[2], raw[3]],
        [raw[4], raw[5], raw[6], raw[7]],
        [raw[8], raw[9], raw[10], raw[11]],
        [raw[12], raw[13], raw[14], raw[15]],
    ]
}

#[inline]
fn next_generation(generation: u32) -> u32 {
    let next = generation.wrapping_add(1);
    if next == 0 { 1 } else { next }
}

#[cfg(test)]
mod tests {
    use super::*;
    use newengine_materials::{MaterialDescriptor, MaterialRegistry};

    #[test]
    fn gpu_table_record_sizes_are_explicit_shader_abi() {
        assert_eq!(core::mem::size_of::<GpuGeometryRecord>(), 64);
        assert_eq!(core::mem::size_of::<GpuMaterialRecord>(), 96);
        assert_eq!(core::mem::size_of::<GpuObjectRecord>(), 128);
    }

    #[test]
    fn material_slot_stays_stable_across_descriptor_update() {
        let registry = MaterialRegistry::new();
        let id = registry.register_named(
            "gpu.table.material",
            MaterialDescriptor {
                roughness: 0.25,
                ..MaterialDescriptor::default()
            },
        );
        let mut table = StableMaterialTable::default();
        table.synchronize(&registry);
        let first = table.handle(id).unwrap();
        let first_record = table.slots[first.slot as usize].record;
        assert!((first_record.params[1] - 0.25).abs() < 1.0e-6);
        assert_eq!(first_record.texture_indices, [INVALID_BINDLESS_INDEX; 4]);

        registry.upsert_named(
            "gpu.table.material",
            MaterialDescriptor {
                roughness: 0.8,
                ..MaterialDescriptor::default()
            },
        );
        table.synchronize(&registry);
        let updated = table.handle(id).unwrap();
        let updated_record = table.slots[updated.slot as usize].record;
        assert_eq!(updated, first);
        assert!((updated_record.params[1] - 0.8).abs() < 1.0e-6);
    }

    #[test]
    fn material_remove_invalidates_generation_before_slot_reuse() {
        let registry = MaterialRegistry::new();
        let id = registry.register_named("gpu.table.retire", MaterialDescriptor::default());
        let mut table = StableMaterialTable::default();
        table.synchronize(&registry);
        let old = table.handle(id).unwrap();

        registry.remove(id).unwrap();
        table.synchronize(&registry);
        assert!(table.handle(id).is_none());
        assert_eq!(table.slots[old.slot as usize].generation, old.generation + 1);
        assert_eq!(table.slots[old.slot as usize].record.meta[1], 0);
    }

    #[test]
    fn object_slot_reuse_carries_new_generation() {
        let mut table = StableObjectTable::default();
        let first = table.allocate(100);
        table.retire(100);
        assert!(table.handle(100).is_none());
        let second = table.allocate(200);
        assert_eq!(second.slot, first.slot);
        assert_ne!(second.generation, first.generation);
    }

    fn test_material_record(flags: u32) -> GpuMaterialRecord {
        GpuMaterialRecord {
            meta: [
                1,
                MATERIAL_FLAG_ACTIVE | flags,
                MaterialDomain::Surface as u32,
                ShadingModel::PbrMetallicRoughness as u32,
            ],
            ..GpuMaterialRecord::default()
        }
    }

    #[test]
    fn gbuffer_eligibility_requires_receive_shadows_and_constants_only() {
        let receive = test_material_record(MaterialFlags::RECEIVE_SHADOWS.0);
        assert!(receive.gbuffer_constants_compatible());

        let cast_only = test_material_record(MaterialFlags::CAST_SHADOWS.0);
        assert!(!cast_only.gbuffer_constants_compatible());

        let textured = test_material_record(
            MaterialFlags::RECEIVE_SHADOWS.0 | MATERIAL_FLAG_TEXTURE_BINDINGS,
        );
        assert!(!textured.gbuffer_constants_compatible());

        let alpha = test_material_record(
            MaterialFlags::RECEIVE_SHADOWS.0 | MaterialFlags::ALPHA_TEST.0,
        );
        assert!(!alpha.gbuffer_constants_compatible());
    }

    #[test]
    fn shadow_eligibility_requires_cast_and_rejects_alpha_or_double_sided() {
        let caster = test_material_record(MaterialFlags::CAST_SHADOWS.0);
        assert!(caster.shadow_opaque_compatible());

        let receive_only = test_material_record(MaterialFlags::RECEIVE_SHADOWS.0);
        assert!(!receive_only.shadow_opaque_compatible());

        let masked = test_material_record(
            MaterialFlags::CAST_SHADOWS.0 | MaterialFlags::ALPHA_TEST.0,
        );
        assert!(!masked.shadow_opaque_compatible());

        let double_sided = test_material_record(
            MaterialFlags::CAST_SHADOWS.0 | MaterialFlags::DOUBLE_SIDED.0,
        );
        assert!(!double_sided.shadow_opaque_compatible());
    }

    #[test]
    fn generation_never_exposes_zero() {
        assert_eq!(next_generation(u32::MAX), 1);
    }
}

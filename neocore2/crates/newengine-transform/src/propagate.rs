#![forbid(unsafe_op_in_unsafe_fn)]

use crate::{GlobalTransform, Parent, Transform, TransformDirty, WorldPose};
use newengine_ecs::{EntityId, World};
use newengine_math::collections_prelude::NeKey;
use newengine_math::{EulerRot, Mat4};
use newengine_transform_api::EntityHandle;

/// Reusable scratch buffers for transform propagation.
///
/// This resource exists to keep the hot path allocation-free.
#[derive(Default)]
pub struct TransformPropagationScratch {
    pub ids: Vec<EntityId>,
    pub locals: Vec<Mat4>,
    pub parents: Vec<Option<EntityHandle>>,
    pub vis: Vec<u8>,
    pub out: Vec<Mat4>,
    pub stack: Vec<(usize, u8)>,
    pub ensure_ids: Vec<EntityId>,
}

/// Per-propagation journal of entities whose published world matrix actually changed.
///
/// This is intentionally a reusable ECS resource rather than an event allocation. High-fanout
/// consumers such as rendering can first compare `tick`, then update only these entities instead
/// of scanning every GlobalTransform in a mostly static world.
#[derive(Default)]
pub struct TransformPropagationChanges {
    pub tick: u64,
    pub generation: u64,
    pub entities: Vec<EntityId>,
}

/// Ensures derived outputs (`GlobalTransform`, `WorldPose`) exist for all entities with `Transform`.
///
/// Call once per frame before propagation (or on-demand when authoring).
#[inline]
pub fn ensure_transform_outputs(world: &mut World) {
    // Move scratch out to avoid borrow conflicts.
    let mut scratch =
        core::mem::take(world.resource_mut_or_insert_default::<TransformPropagationScratch>());

    scratch.ensure_ids.clear();
    scratch
        .ensure_ids
        .extend(world.query::<Transform>().map(|(id, _)| id));
    scratch
        .ensure_ids
        .sort_unstable_by_key(|e| e.data().as_ffi());
    scratch.ensure_ids.dedup();

    for id in scratch.ensure_ids.iter().copied() {
        if world.get::<GlobalTransform>(id).is_none() {
            let _ = world.insert(id, GlobalTransform::default());
        }
        if world.get::<WorldPose>(id).is_none() {
            let _ = world.insert(id, WorldPose::default());
        }
    }

    *world.resource_mut_or_insert_default::<TransformPropagationScratch>() = scratch;
}

/// Propagates `Transform` + hierarchy into `GlobalTransform` and `WorldPose`.
///
/// Properties:
/// - deterministic for a fixed World state (stable ordering by EntityId)
/// - cycle-safe (cycles degrade to local-space roots)
/// - tolerant to broken parents (missing parent treated as root)
/// - allocation-free hot path (scratch is reused)
#[inline]
pub fn propagate_transforms(world: &mut World) {
    ensure_transform_outputs(world);

    // Move scratch out of the World to avoid borrow conflicts with queries/gets.
    let mut scratch =
        core::mem::take(world.resource_mut_or_insert_default::<TransformPropagationScratch>());

    // 1) Collect all entities that have Transform, deterministically ordered.
    scratch.ids.clear();
    scratch
        .ids
        .extend(world.query::<Transform>().map(|(id, _)| id));
    scratch.ids.sort_unstable_by_key(|e| e.data().as_ffi());
    scratch.ids.dedup();

    if scratch.ids.is_empty() {
        *world.resource_mut_or_insert_default::<TransformPropagationScratch>() = scratch;
        return;
    }

    // 2) Build locals and parents.
    scratch.locals.clear();
    scratch.locals.reserve(scratch.ids.len());
    for &id in scratch.ids.iter() {
        let local = world
            .get::<Transform>(id)
            .copied()
            .unwrap_or_default()
            .to_mat4();
        scratch.locals.push(local);
    }

    scratch.parents.clear();
    scratch.parents.reserve(scratch.ids.len());
    for &id in scratch.ids.iter() {
        scratch.parents.push(world.get::<Parent>(id).map(|p| p.0));
    }

    // 3) Iterative DFS with 3-state visitation.
    // 0 = Unvisited, 1 = Visiting (in stack), 2 = Done
    scratch.vis.clear();
    scratch.vis.resize(scratch.ids.len(), 0);

    scratch.out.clear();
    scratch.out.resize(scratch.ids.len(), Mat4::IDENTITY);

    scratch.stack.clear();
    scratch.stack.reserve(scratch.ids.len() * 2);

    for start in 0..scratch.ids.len() {
        if scratch.vis[start] != 0 {
            continue;
        }

        scratch.stack.push((start, 0));

        while let Some((i, phase)) = scratch.stack.pop() {
            match phase {
                0 => {
                    match scratch.vis[i] {
                        2 => continue,
                        1 => {
                            // Cycle edge reached: degrade to local as root.
                            scratch.out[i] = scratch.locals[i];
                            scratch.vis[i] = 2;
                            continue;
                        }
                        _ => {}
                    }

                    scratch.vis[i] = 1;
                    scratch.stack.push((i, 1));

                    if let Some(pid) = scratch.parents[i] {
                        if let Ok(pidx) = scratch
                            .ids
                            .binary_search_by_key(&pid.stable_id, |id| id.stable_u64())
                        {
                            if scratch.vis[pidx] != 2 {
                                scratch.stack.push((pidx, 0));
                            }
                        }
                    }
                }
                _ => {
                    let local = scratch.locals[i];
                    let composed = if let Some(pid) = scratch.parents[i] {
                        if let Ok(pidx) = scratch
                            .ids
                            .binary_search_by_key(&pid.stable_id, |id| id.stable_u64())
                        {
                            if scratch.vis[pidx] == 2 {
                                scratch.out[pidx] * local
                            } else {
                                local
                            }
                        } else {
                            local
                        }
                    } else {
                        local
                    };

                    scratch.out[i] = composed;
                    scratch.vis[i] = 2;
                }
            }
        }
    }

    let mut published_changes =
        core::mem::take(world.resource_mut_or_insert_default::<TransformPropagationChanges>());
    published_changes.tick = world.tick();
    published_changes.generation = published_changes.generation.saturating_add(1).max(1);
    published_changes.entities.clear();

    // 4) Write-back.
    //
    // Derived transforms are high-fanout inputs for rendering, bounds, shadows, audio and
    // streaming. Marking every GlobalTransform/WorldPose as changed on every frame makes a
    // static world look fully dynamic to all downstream systems. Recompute remains conservative
    // for now, but publish change-tracking only when the derived value actually differs.
    for (i, &id) in scratch.ids.iter().enumerate() {
        let m = scratch.out[i];
        let global_changed = world
            .get::<GlobalTransform>(id)
            .map(|gt| gt.0.to_cols_array() != m.to_cols_array())
            .unwrap_or(true);
        if global_changed {
            if let Some(gt) = world.get_mut_tracked::<GlobalTransform>(id) {
                gt.0 = m;
                published_changes.entities.push(id);
            }
        }

        let (scale, rot, trans) = m.to_scale_rotation_translation();
        let (yaw, pitch, roll) = rot.to_euler(EulerRot::YXZ);
        let pose_changed = world
            .get::<WorldPose>(id)
            .map(|wp| {
                wp.world_pos != trans
                    || wp.yaw != yaw
                    || wp.pitch != pitch
                    || wp.roll != roll
                    || wp.world_scale != scale
            })
            .unwrap_or(true);
        if pose_changed {
            if let Some(wp) = world.get_mut_tracked::<WorldPose>(id) {
                wp.world_pos = trans;
                wp.yaw = yaw;
                wp.pitch = pitch;
                wp.roll = roll;
                wp.world_scale = scale;
            }
        }

        let _ = world.remove::<TransformDirty>(id);
    }

    // Put reusable journals/scratch back.
    *world.resource_mut_or_insert_default::<TransformPropagationChanges>() = published_changes;
    *world.resource_mut_or_insert_default::<TransformPropagationScratch>() = scratch;
}

#[cfg(test)]
mod tests {
    use super::*;
    use newengine_math::Vec3;

    #[test]
    fn unchanged_static_transform_does_not_republish_derived_change_ticks() {
        let mut world = World::new();
        let entity = world.spawn();
        assert!(world.insert(entity, Transform::default()));
        propagate_transforms(&mut world);
        let baseline_tick = world.tick();

        world.advance_tick();
        propagate_transforms(&mut world);

        assert!(
            !world.any_changed_since::<GlobalTransform>(baseline_tick),
            "unchanged GlobalTransform must stay clean across propagation"
        );
        assert!(
            !world.any_changed_since::<WorldPose>(baseline_tick),
            "unchanged WorldPose must stay clean across propagation"
        );
        let changes = world.resource::<TransformPropagationChanges>().unwrap();
        assert_eq!(changes.tick, world.tick());
        assert_eq!(changes.generation, 2);
        assert!(changes.entities.is_empty());
    }

    #[test]
    fn real_transform_motion_republishes_derived_change_ticks() {
        let mut world = World::new();
        let entity = world.spawn();
        assert!(world.insert(entity, Transform::default()));
        propagate_transforms(&mut world);
        let baseline_tick = world.tick();

        world.advance_tick();
        world.get_mut::<Transform>(entity).unwrap().position = Vec3::new(3.0, 2.0, -1.0);
        propagate_transforms(&mut world);

        assert!(world.any_changed_since::<GlobalTransform>(baseline_tick));
        assert!(world.any_changed_since::<WorldPose>(baseline_tick));
        let translation = world
            .get::<GlobalTransform>(entity)
            .unwrap()
            .0
            .transform_point3(Vec3::ZERO);
        assert!((translation - Vec3::new(3.0, 2.0, -1.0)).length() < 1.0e-6);
        let changes = world.resource::<TransformPropagationChanges>().unwrap();
        assert_eq!(changes.tick, world.tick());
        assert_eq!(changes.generation, 2);
        assert_eq!(changes.entities.as_slice(), &[entity]);
    }
}

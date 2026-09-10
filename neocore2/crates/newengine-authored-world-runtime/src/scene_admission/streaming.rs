mod decode;

use super::spawn::{
    spawn_box_collision_ydd_prefab_from_decoded, spawn_collision_ydd_prefab_from_decoded,
    spawn_dynamic_ydd_prefab_from_decoded, spawn_static_ydd_prefab_from_decoded,
};
use super::*;
use super::{
    AuthoredStaticWorldSpawnSummary, BOX_COLLISION_WORLD_PROXY, COLLISION_WORLD_PROXY,
    DYNAMIC_WORLD_PROXY, STATIC_WORLD_PROXY,
};
use newengine_core::{TaskTicket, ThreadPoolHandle};
use newengine_engine_runtime::gameplay::{WorldActivationState, WorldAssemblyProgress};
use newengine_model_runtime::ydd_runtime::{
    ydd_dictionary_ref, DecodedRuntimeYddMeshPart as DecodedPrefabMeshPart,
};
use parking_lot::Mutex;
use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::sync::Arc;
use std::time::Instant;

type StaticWorldDecodePacket = BTreeMap<String, Vec<DecodedPrefabMeshPart>>;
type StaticWorldDecodeResult = Arc<Mutex<Option<Result<StaticWorldDecodePacket, String>>>>;

struct StaticWorldDecodeJob {
    ticket: TaskTicket,
    result: StaticWorldDecodeResult,
    sources: Vec<String>,
}

#[derive(Default)]
struct StaticWorldSourceQueue {
    collision: VecDeque<AuthoredWorldPlacementSpec>,
    visual: VecDeque<AuthoredWorldPlacementSpec>,
}

#[derive(Default)]
struct StaticWorldDecodeQueue {
    backlog: VecDeque<String>,
    queued: BTreeSet<String>,
}

impl StaticWorldDecodeQueue {
    #[inline]
    fn contains(&self, dictionary: &str) -> bool {
        self.queued.contains(dictionary)
    }

    fn push_if_absent(&mut self, dictionary: String) -> bool {
        if !self.queued.insert(dictionary.clone()) {
            return false;
        }
        self.backlog.push_back(dictionary);
        true
    }

    fn pop_front(&mut self) -> Option<String> {
        let dictionary = self.backlog.pop_front()?;
        self.queued.remove(&dictionary);
        Some(dictionary)
    }

    #[inline]
    fn len(&self) -> usize {
        self.backlog.len()
    }
}

struct AuthoredStaticWorldStreamingState {
    parent: EntityId,
    /// Placements are grouped by normalized YDD source so readiness never scans the full world.
    pending_by_source: BTreeMap<String, StaticWorldSourceQueue>,
    /// Physical YDD dictionaries queued for decode. Membership is tracked separately so
    /// incremental enqueue/requeue never scans the FIFO backlog.
    decode_queue: StaticWorldDecodeQueue,
    /// Reverse ownership collapses many `YDD@entry` sources onto one physical dictionary.
    sources_by_dictionary: BTreeMap<String, BTreeSet<String>>,
    /// Terminal sources ready for bounded ECS admission. Collision always wins over visual.
    ready_collision_sources: BTreeSet<String>,
    ready_visual_sources: BTreeSet<String>,
    pending_count: usize,
    decoded_cache: BTreeMap<String, Arc<Vec<DecodedPrefabMeshPart>>>,
    decode_jobs: BTreeMap<String, StaticWorldDecodeJob>,
    decode_errors: BTreeMap<String, String>,
    summary: AuthoredStaticWorldSpawnSummary,
    started_at: Instant,
}

pub fn begin_static_world_prefabs(
    world: &mut newengine_ecs::World,
    _mats: &MaterialRegistry,
    parent: EntityId,
    prefabs: &[AuthoredWorldPlacementSpec],
) -> AuthoredStaticWorldSpawnSummary {
    let mut candidates = prefabs
        .iter()
        .filter(|prefab| {
            prefab.enabled
                && (prefab.proxy.trim().eq_ignore_ascii_case(STATIC_WORLD_PROXY)
                    || prefab
                        .proxy
                        .trim()
                        .eq_ignore_ascii_case(DYNAMIC_WORLD_PROXY)
                    || is_collision_proxy(prefab.proxy.trim()))
        })
        .cloned()
        .collect::<Vec<_>>();
    // Canonicalize the source path once. The old path normalized it repeatedly inside
    // queue scans, ready checks and release checks, creating transient Strings proportional
    // to the number of pending placements.
    for prefab in &mut candidates {
        prefab.source = prefab.source.trim().replace('\\', "/");
    }

    // Collision is launch-critical and is admitted before render-only static geometry.
    // Decoded source packets remain cached for the later visual declaration.
    candidates.sort_by(|a, b| {
        let a_collision = is_simulation_proxy(a.proxy.trim());
        let b_collision = is_simulation_proxy(b.proxy.trim());
        // Collision is launch-critical: admit it before render-only static geometry.
        // Within the same role retain deterministic source order for cache locality.
        b_collision
            .cmp(&a_collision)
            .then_with(|| a.source.cmp(&b.source))
    });

    let total = candidates.len() as u32;
    let pending_count = candidates.len();
    let mut pending_by_source = BTreeMap::<String, StaticWorldSourceQueue>::new();
    let mut decode_queue = StaticWorldDecodeQueue::default();
    let mut sources_by_dictionary = BTreeMap::<String, BTreeSet<String>>::new();
    for prefab in candidates {
        let source = prefab.source.clone();
        let dictionary = ydd_dictionary_ref(&source).unwrap_or_else(|_| source.clone());
        sources_by_dictionary
            .entry(dictionary.clone())
            .or_default()
            .insert(source.clone());
        decode_queue.push_if_absent(dictionary);
        let queue = pending_by_source.entry(source).or_default();
        if is_simulation_proxy(prefab.proxy.trim()) {
            queue.collision.push_back(prefab);
        } else {
            queue.visual.push_back(prefab);
        }
    }

    world.insert_resource(WorldAssemblyProgress {
        total,
        pending: total,
        ..WorldAssemblyProgress::default()
    });
    if total == 0 {
        return AuthoredStaticWorldSpawnSummary::default();
    }

    let dictionary_count = decode_queue.len();
    world.insert_resource(AuthoredStaticWorldStreamingState {
        parent,
        pending_by_source,
        decode_queue,
        sources_by_dictionary,
        ready_collision_sources: BTreeSet::new(),
        ready_visual_sources: BTreeSet::new(),
        pending_count,
        decoded_cache: BTreeMap::new(),
        decode_jobs: BTreeMap::new(),
        decode_errors: BTreeMap::new(),
        summary: AuthoredStaticWorldSpawnSummary::default(),
        started_at: Instant::now(),
    });
    newengine_ulog_api::ulog::info!(
        "static world bootstrap queued models={} dictionaries={} policy='parallel dictionary-aware YDD decode on engine.threading; bounded ECS/GPU admission'",
        total,
        dictionary_count
    );
    AuthoredStaticWorldSpawnSummary {
        models: total,
        ..AuthoredStaticWorldSpawnSummary::default()
    }
}

pub(super) fn enqueue_static_world_prefabs(
    world: &mut newengine_ecs::World,
    mats: &MaterialRegistry,
    parent: EntityId,
    prefabs: &[AuthoredWorldPlacementSpec],
) {
    if prefabs.is_empty() {
        return;
    }
    let Some(mut state) = world.remove_resource::<AuthoredStaticWorldStreamingState>() else {
        let _ = begin_static_world_prefabs(world, mats, parent, prefabs);
        return;
    };

    let mut candidates = prefabs
        .iter()
        .filter(|prefab| {
            prefab.enabled
                && (prefab.proxy.trim().eq_ignore_ascii_case(STATIC_WORLD_PROXY)
                    || prefab
                        .proxy
                        .trim()
                        .eq_ignore_ascii_case(DYNAMIC_WORLD_PROXY)
                    || is_collision_proxy(prefab.proxy.trim()))
        })
        .cloned()
        .collect::<Vec<_>>();
    candidates.sort_by(|a, b| {
        is_simulation_proxy(b.proxy.trim())
            .cmp(&is_simulation_proxy(a.proxy.trim()))
            .then_with(|| a.source.cmp(&b.source))
            .then_with(|| a.id.cmp(&b.id))
    });

    let added = candidates.len();
    for mut prefab in candidates {
        prefab.source = prefab.source.trim().replace('\\', "/");
        let source = prefab.source.clone();
        let dictionary = ydd_dictionary_ref(&source).unwrap_or_else(|_| source.clone());
        state
            .sources_by_dictionary
            .entry(dictionary.clone())
            .or_default()
            .insert(source.clone());
        let source_terminal =
            state.decoded_cache.contains_key(&source) || state.decode_errors.contains_key(&source);
        let dictionary_scheduled =
            state.decode_jobs.contains_key(&dictionary) || state.decode_queue.contains(&dictionary);
        if !source_terminal && !dictionary_scheduled {
            state.decode_queue.push_if_absent(dictionary);
        }
        let queue = state.pending_by_source.entry(source).or_default();
        if is_collision_proxy(prefab.proxy.trim()) {
            queue.collision.push_back(prefab);
        } else {
            queue.visual.push_back(prefab);
        }
    }
    state.pending_count = state.pending_count.saturating_add(added);
    if let Some(progress) = world.resource_mut::<WorldAssemblyProgress>() {
        progress.total = progress
            .total
            .saturating_add(added.min(u32::MAX as usize) as u32);
        progress.pending = progress
            .pending
            .saturating_add(added.min(u32::MAX as usize) as u32);
    }
    newengine_ulog_api::ulog::debug!(
        "static world streaming enqueue placements={} pending={} sources={} dictionaries={}",
        added,
        state.pending_count,
        state.pending_by_source.len(),
        state.sources_by_dictionary.len(),
    );
    world.insert_resource(state);
}

pub(super) fn cancel_static_world_cell_domain(
    world: &mut newengine_ecs::World,
    map_ref: &str,
    coord: newengine_assets_api::MapCellCoordV1,
    domain: super::authored_map_streaming::AuthoredCellDomain,
) -> usize {
    let Some(mut state) = world.remove_resource::<AuthoredStaticWorldStreamingState>() else {
        return 0;
    };
    let mut removed = 0usize;
    let sources = state.pending_by_source.keys().cloned().collect::<Vec<_>>();
    for source in sources {
        let Some(queue) = state.pending_by_source.get_mut(&source) else {
            continue;
        };
        let before = queue.collision.len().saturating_add(queue.visual.len());
        let keep = |prefab: &AuthoredWorldPlacementSpec| {
            !(prefab.authored_map_ref == map_ref
                && prefab.authored_cell == Some(coord)
                && super::authored_map_streaming::static_world_prefab_domain(prefab) == domain)
        };
        queue.collision.retain(&keep);
        queue.visual.retain(keep);
        let after = queue.collision.len().saturating_add(queue.visual.len());
        removed = removed.saturating_add(before.saturating_sub(after));
        if queue.collision.is_empty() && queue.visual.is_empty() {
            state.pending_by_source.remove(&source);
            state.ready_collision_sources.remove(&source);
            state.ready_visual_sources.remove(&source);
            release_static_world_source_packet(&mut state, &source);
        }
    }
    state.pending_count = state.pending_count.saturating_sub(removed);
    if let Some(progress) = world.resource_mut::<WorldAssemblyProgress>() {
        progress.pending = progress
            .pending
            .saturating_sub(removed.min(u32::MAX as usize) as u32);
    }
    world.insert_resource(state);
    removed
}

#[inline]
fn is_collision_proxy(proxy: &str) -> bool {
    proxy.eq_ignore_ascii_case(COLLISION_WORLD_PROXY)
        || proxy.eq_ignore_ascii_case(BOX_COLLISION_WORLD_PROXY)
}

#[inline]
fn is_simulation_proxy(proxy: &str) -> bool {
    is_collision_proxy(proxy) || proxy.eq_ignore_ascii_case(DYNAMIC_WORLD_PROXY)
}

fn mark_static_world_source_terminal(state: &mut AuthoredStaticWorldStreamingState, source: &str) {
    let Some(queue) = state.pending_by_source.get(source) else {
        return;
    };
    if !queue.collision.is_empty() {
        state.ready_collision_sources.insert(source.to_owned());
    } else if !queue.visual.is_empty() {
        state.ready_visual_sources.insert(source.to_owned());
    }
}

fn pop_static_world_admittable(
    state: &mut AuthoredStaticWorldStreamingState,
) -> Option<(String, AuthoredWorldPlacementSpec, bool)> {
    let (source, collision_tier) = if let Some(source) = state.ready_collision_sources.first() {
        (source.clone(), true)
    } else {
        (state.ready_visual_sources.first()?.clone(), false)
    };

    let queue = state.pending_by_source.get_mut(&source)?;
    let prefab = if collision_tier {
        queue.collision.pop_front()
    } else {
        queue.visual.pop_front()
    }?;
    state.pending_count = state.pending_count.saturating_sub(1);

    if collision_tier && queue.collision.is_empty() {
        state.ready_collision_sources.remove(&source);
        if !queue.visual.is_empty() {
            state.ready_visual_sources.insert(source.clone());
        }
    } else if !collision_tier && queue.visual.is_empty() {
        state.ready_visual_sources.remove(&source);
    }

    let source_finished = queue.collision.is_empty() && queue.visual.is_empty();
    if source_finished {
        state.pending_by_source.remove(&source);
        state.ready_collision_sources.remove(&source);
        state.ready_visual_sources.remove(&source);
    }
    Some((source, prefab, source_finished))
}

#[inline]
fn release_static_world_source_packet(state: &mut AuthoredStaticWorldStreamingState, source: &str) {
    state.decoded_cache.remove(source);
    state.decode_errors.remove(source);
    let dictionary = ydd_dictionary_ref(source).unwrap_or_else(|_| source.to_owned());
    let remove_dictionary = if let Some(sources) = state.sources_by_dictionary.get_mut(&dictionary)
    {
        sources.remove(source);
        sources.is_empty()
    } else {
        false
    };
    if remove_dictionary {
        state.sources_by_dictionary.remove(&dictionary);
        if let Some(job) = state.decode_jobs.remove(&dictionary) {
            let _ = job.ticket.cancel();
        }
    }
}

fn static_world_admission_limits(world: &newengine_ecs::World) -> (usize, f32) {
    let prelaunch = world
        .resource::<WorldActivationState>()
        .is_some_and(WorldActivationState::needs_prelaunch_gate);
    if prelaunch {
        let max_models = newengine_runtime_env::var_u32(
            "NEWENGINE_STATIC_WORLD_PRELAUNCH_MODELS_PER_FRAME",
            64,
            1,
            256,
        ) as usize;
        let budget_ms = newengine_runtime_env::var_f32(
            "NEWENGINE_STATIC_WORLD_PRELAUNCH_BUDGET_MS",
            24.0,
            1.0,
            50.0,
        );
        (max_models, budget_ms)
    } else {
        let max_models = newengine_runtime_env::var_u32(
            "NEWENGINE_STATIC_WORLD_BOOTSTRAP_MODELS_PER_FRAME",
            8,
            1,
            32,
        ) as usize;
        let budget_ms = newengine_runtime_env::var_f32(
            "NEWENGINE_STATIC_WORLD_BOOTSTRAP_BUDGET_MS",
            3.5,
            0.5,
            16.0,
        );
        (max_models, budget_ms)
    }
}

pub fn tick_authored_static_world_prefabs(
    world: &mut newengine_ecs::World,
    prims: &mut PrimitiveRegistry,
    mats: &MaterialRegistry,
    thread_pool: Option<&ThreadPoolHandle>,
) {
    let Some(mut state) = world.remove_resource::<AuthoredStaticWorldStreamingState>() else {
        return;
    };
    let launch_critical = world
        .resource::<WorldActivationState>()
        .is_some_and(WorldActivationState::needs_prelaunch_gate);
    if let Some(thread_pool) = thread_pool {
        decode::submit_static_world_decode_jobs(&mut state, thread_pool, launch_critical);
        decode::poll_static_world_decode_jobs(&mut state);
    } else {
        decode::decode_one_static_world_dictionary_synchronously(&mut state);
    }

    let (max_models, admission_budget_ms) = static_world_admission_limits(world);
    let admission_started = Instant::now();

    let mut completed_this_frame = 0u32;
    let mut failed_this_frame = 0u32;
    for _ in 0..max_models {
        let admitted = completed_this_frame.saturating_add(failed_this_frame);
        if admitted > 0 && admission_started.elapsed().as_secs_f32() * 1000.0 >= admission_budget_ms
        {
            break;
        }
        let Some((source, prefab, source_finished)) = pop_static_world_admittable(&mut state)
        else {
            break;
        };
        if let Some(error) = state.decode_errors.get(source.as_str()).cloned() {
            failed_this_frame = failed_this_frame.saturating_add(1);
            newengine_ulog_api::ulog::error!(
                "static world prefab decode failed id='{}' source='{}' err='{}'",
                prefab.id,
                prefab.source,
                error,
            );
            if source_finished {
                release_static_world_source_packet(&mut state, &source);
            }
            continue;
        }
        let Some(decoded) = state.decoded_cache.get(source.as_str()).cloned() else {
            failed_this_frame = failed_this_frame.saturating_add(1);
            newengine_ulog_api::ulog::error!(
                "static world prefab admission lost decoded source id='{}' source='{}'",
                prefab.id,
                prefab.source,
            );
            if source_finished {
                release_static_world_source_packet(&mut state, &source);
            }
            continue;
        };
        let prefab_parent = super::authored_map_streaming::static_world_parent_for_prefab(
            world,
            state.parent,
            &prefab,
        );

        let result = if prefab
            .proxy
            .trim()
            .eq_ignore_ascii_case(BOX_COLLISION_WORLD_PROXY)
        {
            spawn_box_collision_ydd_prefab_from_decoded(
                world,
                mats,
                prefab_parent,
                &prefab,
                decoded.as_slice(),
            )
        } else if prefab
            .proxy
            .trim()
            .eq_ignore_ascii_case(COLLISION_WORLD_PROXY)
        {
            spawn_collision_ydd_prefab_from_decoded(
                world,
                mats,
                prefab_parent,
                &prefab,
                decoded.as_slice(),
            )
        } else if prefab
            .proxy
            .trim()
            .eq_ignore_ascii_case(DYNAMIC_WORLD_PROXY)
        {
            spawn_dynamic_ydd_prefab_from_decoded(
                world,
                prims,
                mats,
                prefab_parent,
                &prefab,
                decoded.as_slice(),
            )
        } else {
            spawn_static_ydd_prefab_from_decoded(
                world,
                prims,
                mats,
                prefab_parent,
                &prefab,
                decoded.as_slice(),
            )
        };
        match result {
            Ok((parts, triangles)) => {
                if !is_collision_proxy(prefab.proxy.trim()) {
                    super::authored_map_streaming::record_static_world_primitive_residency(
                        world,
                        &prefab,
                        decoded.as_slice(),
                    );
                }
                state.summary.models = state.summary.models.saturating_add(1);
                state.summary.parts = state.summary.parts.saturating_add(parts);
                state.summary.triangles = state.summary.triangles.saturating_add(triangles);
                completed_this_frame = completed_this_frame.saturating_add(1);
                newengine_ulog_api::ulog::debug!(
                    "static world prefab streamed id='{}' source='{}' material='{}' position={:?} parts={} triangles={} pending={} decode_jobs={} decoded_ready={}",
                    prefab.id,
                    prefab.source,
                    prefab.material,
                    prefab.position,
                    parts,
                    triangles,
                    state.pending_count,
                    state.decode_jobs.len(),
                    state.decoded_cache.len(),
                );
            }
            Err(error) => {
                failed_this_frame = failed_this_frame.saturating_add(1);
                newengine_ulog_api::ulog::error!(
                    "static world prefab failed id='{}' source='{}' err='{}'",
                    prefab.id,
                    prefab.source,
                    error,
                );
            }
        }
        if source_finished {
            release_static_world_source_packet(&mut state, &source);
        }
    }

    let pending = state.pending_count.min(u32::MAX as usize) as u32;
    if let Some(residency) = world.resource_mut::<WorldAssemblyProgress>() {
        residency.completed = residency.completed.saturating_add(completed_this_frame);
        residency.failed = residency.failed.saturating_add(failed_this_frame);
        residency.pending = pending;
        residency.parts = state.summary.parts;
        residency.triangles = state.summary.triangles;
    }

    if pending == 0 {
        let elapsed_ms = state.started_at.elapsed().as_secs_f32() * 1000.0;
        newengine_ulog_api::ulog::info!(
            "static world bootstrap completed models={} parts={} triangles={} failed={} elapsed_ms={:.2} policy='incremental; no event-loop starvation'",
            state.summary.models,
            state.summary.parts,
            state.summary.triangles,
            world
                .resource::<WorldAssemblyProgress>()
                .map(WorldAssemblyProgress::failed)
                .unwrap_or(0),
            elapsed_ms,
        );
        let _ = newengine_engine_runtime::world_authoring::validate_scene_objects(
            world,
            "authored-world.static-world-complete",
        );
    } else {
        world.insert_resource(state);
    }
}

#[cfg(test)]
mod decode_queue_tests {
    use super::StaticWorldDecodeQueue;

    #[test]
    fn duplicate_dictionary_is_never_enqueued_twice() {
        let mut queue = StaticWorldDecodeQueue::default();
        let dictionary = char::from(97).to_string();
        assert!(queue.push_if_absent(dictionary.clone()));
        assert!(!queue.push_if_absent(dictionary.clone()));
        assert_eq!(queue.len(), 1);
        assert!(queue.contains(&dictionary));
        assert_eq!(queue.pop_front().as_deref(), Some(dictionary.as_str()));
        assert!(!queue.contains(&dictionary));
    }

    #[test]
    fn popped_dictionary_can_be_requeued_without_losing_fifo_order() {
        let mut queue = StaticWorldDecodeQueue::default();
        let first = char::from(97).to_string();
        let second = char::from(98).to_string();
        assert!(queue.push_if_absent(first.clone()));
        assert!(queue.push_if_absent(second.clone()));
        assert_eq!(queue.pop_front().as_deref(), Some(first.as_str()));
        assert!(queue.push_if_absent(first.clone()));
        assert_eq!(queue.pop_front().as_deref(), Some(second.as_str()));
        assert_eq!(queue.pop_front().as_deref(), Some(first.as_str()));
        assert_eq!(queue.len(), 0);
    }
}

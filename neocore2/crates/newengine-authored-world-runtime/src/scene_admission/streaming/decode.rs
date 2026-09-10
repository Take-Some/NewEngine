#![forbid(unsafe_op_in_unsafe_fn)]

use super::{
    mark_static_world_source_terminal, AuthoredStaticWorldStreamingState, StaticWorldDecodeJob,
};
use newengine_core::{TaskLane, TaskPriority, TaskRequest, ThreadPoolHandle};
use newengine_model_runtime::ydd_runtime::decode_runtime_ydd_prefabs;
use parking_lot::Mutex;
use std::sync::Arc;

#[inline]
fn static_world_decode_priority(launch_critical: bool) -> TaskPriority {
    if launch_critical {
        TaskPriority::Critical
    } else {
        TaskPriority::Interactive
    }
}

fn static_world_decode_concurrency(_thread_pool: &ThreadPoolHandle) -> usize {
    // A decoded YDD can be tens of MiB before ECS/GPU admission. Running several
    // dictionary decodes in parallel makes peak commit scale with job count and can
    // exhaust the system before the ready packets are consumed. Keep the production
    // default conservative; explicit profiling runs may raise it through the env gate.
    newengine_runtime_env::var_u32("NEWENGINE_STATIC_WORLD_DECODE_JOBS", 1, 1, 6) as usize
}

fn static_world_decode_ready_source_limit() -> usize {
    // Backpressure is based on decoded source packets, not only active worker jobs.
    // Without this bound decode can outrun bounded ECS admission and retain hundreds
    // of fully expanded mesh packets at once on dense authored maps.
    newengine_runtime_env::var_u32("NEWENGINE_STATIC_WORLD_DECODE_READY_SOURCES", 1, 1, 32) as usize
}

#[inline]
fn static_world_decode_resident_sources(state: &AuthoredStaticWorldStreamingState) -> usize {
    state.decoded_cache.len().saturating_add(
        state
            .decode_jobs
            .values()
            .map(|job| job.sources.len())
            .sum::<usize>(),
    )
}

fn unresolved_sources_for_dictionary(
    state: &AuthoredStaticWorldStreamingState,
    dictionary: &str,
) -> Vec<String> {
    let unresolved = state
        .sources_by_dictionary
        .get(dictionary)
        .into_iter()
        .flat_map(|sources| sources.iter())
        .filter(|source| {
            state.pending_by_source.contains_key(source.as_str())
                && !state.decoded_cache.contains_key(source.as_str())
                && !state.decode_errors.contains_key(source.as_str())
        })
        .cloned()
        .collect::<Vec<_>>();
    let has_launch_critical = unresolved.iter().any(|source| {
        state
            .pending_by_source
            .get(source)
            .is_some_and(|queue| !queue.collision.is_empty())
    });
    if !has_launch_critical {
        return unresolved;
    }
    unresolved
        .into_iter()
        .filter(|source| {
            state
                .pending_by_source
                .get(source)
                .is_some_and(|queue| !queue.collision.is_empty())
        })
        .collect()
}

fn requeue_dictionary_if_needed(state: &mut AuthoredStaticWorldStreamingState, dictionary: &str) {
    if unresolved_sources_for_dictionary(state, dictionary).is_empty()
        || state.decode_jobs.contains_key(dictionary)
        || state.decode_queue.contains(dictionary)
    {
        return;
    }
    state.decode_queue.push_if_absent(dictionary.to_owned());
}

pub(super) fn submit_static_world_decode_jobs(
    state: &mut AuthoredStaticWorldStreamingState,
    thread_pool: &ThreadPoolHandle,
    launch_critical: bool,
) {
    let max_jobs = static_world_decode_concurrency(thread_pool);
    let ready_source_limit = static_world_decode_ready_source_limit();
    let resident_sources = static_world_decode_resident_sources(state);
    let ready_slots = ready_source_limit.saturating_sub(resident_sources);
    let free_slots = max_jobs
        .saturating_sub(state.decode_jobs.len())
        .min(ready_slots);
    if free_slots == 0 {
        return;
    }

    let mut dictionaries = Vec::with_capacity(free_slots);
    for _ in 0..free_slots {
        let Some(dictionary) = state.decode_queue.pop_front() else {
            break;
        };
        dictionaries.push(dictionary);
    }

    let mut source_slots = ready_slots;
    for dictionary in dictionaries {
        let mut sources = unresolved_sources_for_dictionary(state, &dictionary);
        if sources.is_empty() {
            continue;
        }
        // A physical dictionary can expose multiple selectors. Decode only the number
        // of source packets that can become resident now; the dictionary is requeued
        // after this job so the remaining selectors stream in a later admission wave.
        sources.truncate(source_slots.max(1));
        source_slots = source_slots.saturating_sub(sources.len());

        let worker_sources = sources.clone();
        let result = Arc::new(Mutex::new(None));
        let result_out = Arc::clone(&result);
        let request = TaskRequest::new("static.world.ydd.dictionary.decode")
            .with_source("scene.bridge.game-ready")
            .with_owner("engine.scene")
            .with_category("asset-decode")
            .with_lane(TaskLane::AssetIo)
            .with_priority(static_world_decode_priority(launch_critical))
            .with_task_id(format!(
                "scene.static-world.dictionary.decode.{:016x}",
                newengine_primitives::fnv1a_64(&dictionary)
            ));
        let host_context = newengine_plugin_host::current_host_context();
        let ticket = thread_pool.submit_request(request, move || {
            let decoded = newengine_plugin_host::with_host_context(&host_context, || {
                decode_runtime_ydd_prefabs(&worker_sources)
            });
            *result_out.lock() = Some(decoded);
        });
        state.decode_jobs.insert(
            dictionary,
            StaticWorldDecodeJob {
                ticket,
                result,
                sources,
            },
        );
        if source_slots == 0 {
            break;
        }
    }
}

pub(super) fn poll_static_world_decode_jobs(state: &mut AuthoredStaticWorldStreamingState) {
    let ready = state
        .decode_jobs
        .iter()
        .filter(|(_, job)| job.ticket.is_complete())
        .map(|(dictionary, _)| dictionary.clone())
        .collect::<Vec<_>>();
    for dictionary in ready {
        let Some(job) = state.decode_jobs.remove(&dictionary) else {
            continue;
        };
        let result = job.result.lock().take();
        match result {
            Some(Ok(mut decoded_packet)) => {
                for source in &job.sources {
                    if !state.pending_by_source.contains_key(source) {
                        continue;
                    }
                    match decoded_packet.remove(source) {
                        Some(decoded) => {
                            state
                                .decoded_cache
                                .insert(source.clone(), Arc::new(decoded));
                        }
                        None => {
                            state.decode_errors.insert(
                                source.clone(),
                                format!(
                                    "static world dictionary decode omitted selector dictionary='{}' source='{}'",
                                    dictionary, source
                                ),
                            );
                        }
                    }
                    mark_static_world_source_terminal(state, source);
                }
            }
            Some(Err(error)) => {
                for source in &job.sources {
                    if !state.pending_by_source.contains_key(source) {
                        continue;
                    }
                    state.decode_errors.insert(source.clone(), error.clone());
                    mark_static_world_source_terminal(state, source);
                }
            }
            None => {
                for source in &job.sources {
                    if !state.pending_by_source.contains_key(source) {
                        continue;
                    }
                    state.decode_errors.insert(
                        source.clone(),
                        format!(
                            "static world dictionary decode task completed without result dictionary='{}'",
                            dictionary
                        ),
                    );
                    mark_static_world_source_terminal(state, source);
                }
            }
        }
        requeue_dictionary_if_needed(state, &dictionary);
    }
}

pub(super) fn decode_one_static_world_dictionary_synchronously(
    state: &mut AuthoredStaticWorldStreamingState,
) {
    if static_world_decode_resident_sources(state) >= static_world_decode_ready_source_limit() {
        return;
    }
    let Some(dictionary) = state.decode_queue.pop_front() else {
        return;
    };
    let mut sources = unresolved_sources_for_dictionary(state, &dictionary);
    if sources.is_empty() {
        return;
    }
    let source_slots = static_world_decode_ready_source_limit()
        .saturating_sub(static_world_decode_resident_sources(state))
        .max(1);
    sources.truncate(source_slots);
    match decode_runtime_ydd_prefabs(&sources) {
        Ok(mut decoded_packet) => {
            for source in &sources {
                match decoded_packet.remove(source) {
                    Some(decoded) => {
                        state
                            .decoded_cache
                            .insert(source.clone(), Arc::new(decoded));
                    }
                    None => {
                        state.decode_errors.insert(
                            source.clone(),
                            format!(
                                "static world dictionary decode omitted selector dictionary='{}' source='{}'",
                                dictionary, source
                            ),
                        );
                    }
                }
                mark_static_world_source_terminal(state, source);
            }
        }
        Err(error) => {
            for source in &sources {
                state.decode_errors.insert(source.clone(), error.clone());
                mark_static_world_source_terminal(state, source);
            }
        }
    }
    requeue_dictionary_if_needed(state, &dictionary);
}

#[cfg(test)]
mod priority_tests {
    use super::static_world_decode_priority;
    use newengine_core::TaskPriority;

    #[test]
    fn launch_decode_priority_bypasses_frame_budget_starvation() {
        assert_eq!(static_world_decode_priority(true), TaskPriority::Critical);
        assert_eq!(
            static_world_decode_priority(false),
            TaskPriority::Interactive
        );
    }
}

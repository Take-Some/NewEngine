#![forbid(unsafe_op_in_unsafe_fn)]

use newengine_core::render::Extent2D;
use newengine_math::Vec3;
use newengine_model_domain_api::{MeshRenderRole, MeshTransformPolicy, MeshVisibilityPolicy};
use newengine_visibility_api::{
    decode_visibility_result_batch_bin, encode_visibility_query_batch_bin, visibility_method,
    VisibilityObservationV1, VisibilityQueryBatchV1, VisibilityQueryCandidateV1,
    VisibilityResultBatchV1, VisibilitySphereV1, VisibilityVec3V1, VisibilityViewV1,
    ENGINE_VISIBILITY_SERVICE_ID,
};

use super::passes::mesh_visibility::{
    primitive_forward_max_distance, sphere_screen_coverage_hint, sphere_within_render_distance,
    transform_sphere,
};
use super::RuntimeRenderController;

const VISIBILITY_QUERY_CAP: usize = 4096;
const OCCLUSION_CONFIRMATIONS_REQUIRED: u8 = 2;
const OCCLUSION_CONFIDENCE_MIN: f32 = 0.85;
const OCCLUSION_RESULT_MAX_AGE_FRAMES: u64 = 6;
const OCCLUSION_HISTORY_RETENTION_FRAMES: u64 = 120;
const OCCLUSION_MIN_CULL_DISTANCE_METERS: f32 = 6.0;
const OCCLUSION_MAX_CULL_COVERAGE_HINT: f32 = 0.10;
const CAMERA_CUT_DISTANCE_METERS: f32 = 25.0;
const CAMERA_CUT_FORWARD_DOT: f32 = 0.65;

#[derive(Clone, Copy, Debug)]
struct VisibilityCandidatePlan {
    priority: i32,
    candidate: VisibilityQueryCandidateV1,
}

impl RuntimeRenderController {
    /// Pushes this frame's persistent RenderScene bounds into the provider-neutral visibility
    /// gateway and consumes the oldest already-completed Hi-Z observations.
    ///
    /// The call is intentionally performed once before draw-list extraction. The provider is
    /// asynchronous: current-frame rendering only consumes delayed observations that survived
    /// engine-side motion/camera invalidation and hysteresis.
    pub(super) fn update_world_visibility_control(
        &mut self,
        scene: &newengine_scene::Scene,
        runtime: bool,
        camera_position: Vec3,
        camera_forward: Vec3,
        viewport_extent: Extent2D,
    ) {
        let frame = self.frame.frame_index;
        if self.frame.visibility.last_submission_frame == frame {
            return;
        }
        self.frame.visibility.last_submission_frame = frame;

        if !runtime {
            self.frame.visibility.clear_history();
            return;
        }

        let scene_key = scene as *const newengine_scene::Scene as usize;
        let camera_forward = normalize_or_default(camera_forward);
        let camera_position_raw = vec3_array(camera_position);
        let camera_forward_raw = vec3_array(camera_forward);
        let viewport_raw = [viewport_extent.width, viewport_extent.height];

        let scene_changed = self.frame.visibility.scene_key != Some(scene_key);
        let viewport_changed = self
            .frame
            .visibility
            .last_viewport_extent
            .is_some_and(|previous| previous != viewport_raw);
        let camera_cut = visibility_camera_cut(
            self.frame.visibility.last_camera_position,
            self.frame.visibility.last_camera_forward,
            camera_position_raw,
            camera_forward_raw,
        );
        if scene_changed || viewport_changed || camera_cut {
            self.frame.visibility.clear_history();
        }
        self.frame.visibility.scene_key = Some(scene_key);
        self.frame.visibility.last_camera_position = Some(camera_position_raw);
        self.frame.visibility.last_camera_forward = Some(camera_forward_raw);
        self.frame.visibility.last_viewport_extent = Some(viewport_raw);

        if !newengine_plugin_host::has_service(ENGINE_VISIBILITY_SERVICE_ID) {
            clear_occlusion_confirmations(&mut self.frame.visibility.history);
            self.frame.visibility.last_candidate_count = 0;
            return;
        }

        let (primitive_snapshot, _) = self.primitive_scene_snapshot(scene, runtime);
        let max_distance = primitive_forward_max_distance(runtime).max(1.0);
        let mut candidates = Vec::with_capacity(
            primitive_snapshot
                .entries
                .len()
                .min(VISIBILITY_QUERY_CAP),
        );

        for source in primitive_snapshot.entries.iter() {
            if !visibility_candidate_role(source.render_options.role)
                || source.render_options.transform_policy != MeshTransformPolicy::World
                || !matches!(
                    source.render_options.visibility_policy,
                    MeshVisibilityPolicy::Frustum | MeshVisibilityPolicy::FrustumAndDistance
                )
            {
                continue;
            }
            let Some((local_center, local_radius)) = source.local_bounds else {
                continue;
            };
            let (center_ws, radius_ws) =
                transform_sphere(source.render_model, local_center, local_radius);
            if !sphere_within_render_distance(
                camera_position,
                center_ws,
                radius_ws,
                max_distance,
            ) {
                continue;
            }
            let distance = (center_ws - camera_position).length();
            let coverage = sphere_screen_coverage_hint(radius_ws, distance);
            let priority = visibility_priority(coverage, distance, max_distance);
            update_candidate_motion_history(
                &mut self.frame.visibility.history,
                source.entity_key,
                center_ws,
                radius_ws,
                frame,
            );
            candidates.push(VisibilityCandidatePlan {
                priority,
                candidate: VisibilityQueryCandidateV1 {
                    subject_id: source.entity_key,
                    bounds: VisibilitySphereV1 {
                        center: visibility_vec3(center_ws),
                        radius: radius_ws.max(0.001),
                    },
                    priority,
                },
            });
        }

        trim_visibility_candidates(&mut candidates, VISIBILITY_QUERY_CAP);
        self.frame.visibility.last_candidate_count = candidates.len();
        self.frame.visibility.history.retain(|_, history| {
            frame.saturating_sub(history.last_seen_frame) <= OCCLUSION_HISTORY_RETENTION_FRAMES
        });

        if candidates.is_empty() {
            clear_occlusion_confirmations(&mut self.frame.visibility.history);
            return;
        }

        let query = VisibilityQueryBatchV1 {
            frame,
            view: VisibilityViewV1 {
                position: visibility_vec3(camera_position),
                forward: visibility_vec3(camera_forward),
                max_distance,
                // The Vulkan provider performs the exact camera-space/frustum projection itself.
                // Keep this deliberately wide for future providers that use the coarse hint.
                coarse_cone_dot: -0.25,
            },
            max_results: candidates.len(),
            candidates: candidates
                .into_iter()
                .map(|planned| planned.candidate)
                .collect(),
        };
        let payload = match encode_visibility_query_batch_bin(&query) {
            Ok(payload) => payload,
            Err(error) => {
                self.record_visibility_service_failure(frame, &error);
                return;
            }
        };
        let response = match newengine_core::host_services::call_service_v1(
            ENGINE_VISIBILITY_SERVICE_ID,
            visibility_method::QUERY_BATCH_BIN_V1,
            &payload,
        ) {
            Ok(bytes) => bytes,
            Err(error) => {
                self.record_visibility_service_failure(frame, &error.to_string());
                return;
            }
        };
        let result = match decode_visibility_result_batch_bin(&response) {
            Ok(result) => result,
            Err(error) => {
                self.record_visibility_service_failure(frame, &error);
                return;
            }
        };
        apply_visibility_results(&mut self.frame.visibility, frame, &result);

        if frame <= 3 || frame.is_multiple_of(120) {
            newengine_ulog_api::ulog::debug!(
                "render.visibility.control: frame={} candidates={} results={} provider_frame={} confirmed_occluded={} service_failures={} transport='binary-v1'",
                frame,
                self.frame.visibility.last_candidate_count,
                self.frame.visibility.last_result_count,
                self.frame.visibility.last_provider_frame,
                self.frame
                    .visibility
                    .history
                    .values()
                    .filter(|history| history.confirmed_occluded)
                    .count(),
                self.frame.visibility.service_failures,
            );
        }
    }

    #[inline]
    pub(super) fn visibility_should_cull_world_primitive(
        &self,
        subject_id: u64,
        distance_m: f32,
        screen_coverage_hint: f32,
    ) -> bool {
        should_cull_from_history(
            self.frame.visibility.history.get(&subject_id),
            self.frame.frame_index,
            distance_m,
            screen_coverage_hint,
        )
    }

    fn record_visibility_service_failure(&mut self, frame: u64, error: &str) {
        self.frame.visibility.service_failures =
            self.frame.visibility.service_failures.saturating_add(1);
        clear_occlusion_confirmations(&mut self.frame.visibility.history);
        if frame <= 3 || frame.is_multiple_of(120) {
            newengine_ulog_api::ulog::warn!(
                "render.visibility.control: async provider call failed frame={} failures={} error='{}'; CPU visibility remains conservative",
                frame,
                self.frame.visibility.service_failures,
                error,
            );
        }
    }
}

fn visibility_candidate_role(role: MeshRenderRole) -> bool {
    matches!(
        role,
        MeshRenderRole::WorldOpaque | MeshRenderRole::WorldMasked | MeshRenderRole::FoliageInstanced
    )
}

#[inline]
fn visibility_priority(coverage: f32, distance: f32, max_distance: f32) -> i32 {
    let coverage_score = (coverage.clamp(0.0, 1.0) * 1_000_000.0) as i32;
    let proximity = (1.0 - (distance / max_distance.max(1.0))).clamp(0.0, 1.0);
    coverage_score.saturating_add((proximity * 100_000.0) as i32)
}

fn trim_visibility_candidates(candidates: &mut Vec<VisibilityCandidatePlan>, cap: usize) {
    if cap == 0 {
        candidates.clear();
        return;
    }
    let compare = |a: &VisibilityCandidatePlan, b: &VisibilityCandidatePlan| {
        b.priority
            .cmp(&a.priority)
            .then_with(|| a.candidate.subject_id.cmp(&b.candidate.subject_id))
    };
    if candidates.len() > cap {
        candidates.select_nth_unstable_by(cap, compare);
        candidates.truncate(cap);
    }
    candidates.sort_unstable_by(compare);
}

fn update_candidate_motion_history(
    history: &mut newengine_math::collections::FxHashMap<
        u64,
        crate::render_controller::state::VisibilityHistoryEntry,
    >,
    subject_id: u64,
    center: Vec3,
    radius: f32,
    frame: u64,
) {
    let entry = history.entry(subject_id).or_default();
    let center_raw = vec3_array(center);
    let radius = radius.abs().max(0.001);
    let moved = if entry.initialized_bounds {
        let dx = center_raw[0] - entry.last_center[0];
        let dy = center_raw[1] - entry.last_center[1];
        let dz = center_raw[2] - entry.last_center[2];
        let movement_sq = dx * dx + dy * dy + dz * dz;
        let movement_threshold = (radius * 0.02).max(0.05);
        let radius_threshold = (radius * 0.02).max(0.02);
        movement_sq > movement_threshold * movement_threshold
            || (radius - entry.last_radius).abs() > radius_threshold
    } else {
        true
    };
    if moved {
        entry.last_motion_frame = frame;
        entry.consecutive_occluded = 0;
        entry.confirmed_occluded = false;
        entry.confidence = 0.0;
    }
    entry.last_center = center_raw;
    entry.last_radius = radius;
    entry.last_seen_frame = frame;
    entry.initialized_bounds = true;
}

fn apply_visibility_results(
    state: &mut crate::render_controller::state::RenderVisibilityRuntimeState,
    frame: u64,
    batch: &VisibilityResultBatchV1,
) {
    state.last_provider_frame = batch.provider_frame;
    state.last_result_count = batch.results.len();
    for result in &batch.results {
        let Some(history) = state.history.get_mut(&result.subject_id) else {
            continue;
        };
        if result.produced_frame <= history.last_produced_frame {
            continue;
        }
        let previous_produced_frame = history.last_produced_frame;
        history.last_produced_frame = result.produced_frame;

        if result.produced_frame < history.last_motion_frame
            || frame.saturating_sub(result.produced_frame) > OCCLUSION_RESULT_MAX_AGE_FRAMES
        {
            history.consecutive_occluded = 0;
            history.confirmed_occluded = false;
            history.confidence = 0.0;
            continue;
        }

        let confidence = if result.confidence.is_finite() {
            result.confidence.clamp(0.0, 1.0)
        } else {
            0.0
        };
        match result.observation {
            VisibilityObservationV1::Visible | VisibilityObservationV1::Unknown => {
                history.consecutive_occluded = 0;
                history.confirmed_occluded = false;
                history.confidence = confidence;
            }
            VisibilityObservationV1::Occluded if confidence >= OCCLUSION_CONFIDENCE_MIN => {
                if previous_produced_frame > 0
                    && result.produced_frame > previous_produced_frame.saturating_add(2)
                {
                    history.consecutive_occluded = 0;
                }
                history.consecutive_occluded = history.consecutive_occluded.saturating_add(1);
                history.confirmed_occluded =
                    history.consecutive_occluded >= OCCLUSION_CONFIRMATIONS_REQUIRED;
                history.confidence = confidence;
            }
            VisibilityObservationV1::Occluded => {
                history.consecutive_occluded = 0;
                history.confirmed_occluded = false;
                history.confidence = confidence;
            }
        }
    }
}

#[inline]
fn should_cull_from_history(
    history: Option<&crate::render_controller::state::VisibilityHistoryEntry>,
    frame: u64,
    distance_m: f32,
    screen_coverage_hint: f32,
) -> bool {
    if distance_m <= OCCLUSION_MIN_CULL_DISTANCE_METERS
        || screen_coverage_hint >= OCCLUSION_MAX_CULL_COVERAGE_HINT
    {
        return false;
    }
    let Some(history) = history else {
        return false;
    };
    history.confirmed_occluded
        && history.consecutive_occluded >= OCCLUSION_CONFIRMATIONS_REQUIRED
        && history.confidence >= OCCLUSION_CONFIDENCE_MIN
        && history.last_produced_frame >= history.last_motion_frame
        && frame.saturating_sub(history.last_produced_frame) <= OCCLUSION_RESULT_MAX_AGE_FRAMES
}

fn clear_occlusion_confirmations(
    history: &mut newengine_math::collections::FxHashMap<
        u64,
        crate::render_controller::state::VisibilityHistoryEntry,
    >,
) {
    for entry in history.values_mut() {
        entry.consecutive_occluded = 0;
        entry.confirmed_occluded = false;
        entry.confidence = 0.0;
    }
}

#[inline]
fn visibility_camera_cut(
    previous_position: Option<[f32; 3]>,
    previous_forward: Option<[f32; 3]>,
    current_position: [f32; 3],
    current_forward: [f32; 3],
) -> bool {
    let (Some(previous_position), Some(previous_forward)) =
        (previous_position, previous_forward)
    else {
        return false;
    };
    let dx = current_position[0] - previous_position[0];
    let dy = current_position[1] - previous_position[1];
    let dz = current_position[2] - previous_position[2];
    let distance_sq = dx * dx + dy * dy + dz * dz;
    let direction_dot = previous_forward[0] * current_forward[0]
        + previous_forward[1] * current_forward[1]
        + previous_forward[2] * current_forward[2];
    distance_sq > CAMERA_CUT_DISTANCE_METERS * CAMERA_CUT_DISTANCE_METERS
        || direction_dot < CAMERA_CUT_FORWARD_DOT
}

#[inline]
fn normalize_or_default(value: Vec3) -> Vec3 {
    let len_sq = value.length_squared();
    if !len_sq.is_finite() || len_sq <= 1.0e-8 {
        Vec3::new(0.0, 0.0, -1.0)
    } else {
        value / len_sq.sqrt()
    }
}

#[inline]
fn visibility_vec3(value: Vec3) -> VisibilityVec3V1 {
    VisibilityVec3V1 {
        x: value.x,
        y: value.y,
        z: value.z,
    }
}

#[inline]
fn vec3_array(value: Vec3) -> [f32; 3] {
    [value.x, value.y, value.z]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::render_controller::state::{RenderVisibilityRuntimeState, VisibilityHistoryEntry};
    use newengine_visibility_api::VisibilitySubjectResultV1;

    fn occluded(subject_id: u64, produced_frame: u64) -> VisibilitySubjectResultV1 {
        VisibilitySubjectResultV1 {
            subject_id,
            observation: VisibilityObservationV1::Occluded,
            confidence: 0.90,
            produced_frame,
        }
    }

    #[test]
    fn two_fresh_occlusion_observations_are_required_before_culling() {
        let mut state = RenderVisibilityRuntimeState::new();
        let mut history = VisibilityHistoryEntry::default();
        history.initialized_bounds = true;
        history.last_seen_frame = 10;
        history.last_motion_frame = 8;
        state.history.insert(7, history);

        apply_visibility_results(
            &mut state,
            10,
            &VisibilityResultBatchV1 {
                provider_frame: 10,
                results: vec![occluded(7, 9)],
                diagnostics: Vec::new(),
            },
        );
        assert!(!should_cull_from_history(state.history.get(&7), 10, 20.0, 0.01));

        apply_visibility_results(
            &mut state,
            11,
            &VisibilityResultBatchV1 {
                provider_frame: 11,
                results: vec![occluded(7, 10)],
                diagnostics: Vec::new(),
            },
        );
        assert!(should_cull_from_history(state.history.get(&7), 11, 20.0, 0.01));
    }

    #[test]
    fn visible_observation_immediately_breaks_occlusion_hysteresis() {
        let mut state = RenderVisibilityRuntimeState::new();
        state.history.insert(
            7,
            VisibilityHistoryEntry {
                consecutive_occluded: 2,
                confirmed_occluded: true,
                confidence: 0.9,
                last_produced_frame: 9,
                last_motion_frame: 7,
                last_seen_frame: 10,
                ..VisibilityHistoryEntry::default()
            },
        );
        apply_visibility_results(
            &mut state,
            10,
            &VisibilityResultBatchV1 {
                provider_frame: 10,
                results: vec![VisibilitySubjectResultV1 {
                    subject_id: 7,
                    observation: VisibilityObservationV1::Visible,
                    confidence: 0.85,
                    produced_frame: 10,
                }],
                diagnostics: Vec::new(),
            },
        );
        assert!(!should_cull_from_history(state.history.get(&7), 10, 20.0, 0.01));
    }

    #[test]
    fn motion_rejects_delayed_pre_motion_occlusion() {
        let mut state = RenderVisibilityRuntimeState::new();
        state.history.insert(
            7,
            VisibilityHistoryEntry {
                initialized_bounds: true,
                last_motion_frame: 20,
                last_seen_frame: 20,
                ..VisibilityHistoryEntry::default()
            },
        );
        apply_visibility_results(
            &mut state,
            21,
            &VisibilityResultBatchV1 {
                provider_frame: 21,
                results: vec![occluded(7, 19)],
                diagnostics: Vec::new(),
            },
        );
        let history = state.history.get(&7).unwrap();
        assert!(!history.confirmed_occluded);
        assert_eq!(history.consecutive_occluded, 0);
    }

    #[test]
    fn stale_or_large_near_objects_fail_visible() {
        let history = VisibilityHistoryEntry {
            consecutive_occluded: 3,
            confirmed_occluded: true,
            confidence: 0.95,
            last_produced_frame: 10,
            last_motion_frame: 8,
            last_seen_frame: 20,
            ..VisibilityHistoryEntry::default()
        };
        assert!(!should_cull_from_history(Some(&history), 20, 30.0, 0.01));
        assert!(!should_cull_from_history(Some(&history), 12, 3.0, 0.01));
        assert!(!should_cull_from_history(Some(&history), 12, 30.0, 0.2));
    }

    #[test]
    fn candidate_trim_keeps_highest_priorities_deterministically() {
        let mut candidates = vec![
            VisibilityCandidatePlan {
                priority: 2,
                candidate: VisibilityQueryCandidateV1 {
                    subject_id: 20,
                    bounds: VisibilitySphereV1 {
                        center: VisibilityVec3V1::default(),
                        radius: 1.0,
                    },
                    priority: 2,
                },
            },
            VisibilityCandidatePlan {
                priority: 9,
                candidate: VisibilityQueryCandidateV1 {
                    subject_id: 90,
                    bounds: VisibilitySphereV1 {
                        center: VisibilityVec3V1::default(),
                        radius: 1.0,
                    },
                    priority: 9,
                },
            },
            VisibilityCandidatePlan {
                priority: 9,
                candidate: VisibilityQueryCandidateV1 {
                    subject_id: 80,
                    bounds: VisibilitySphereV1 {
                        center: VisibilityVec3V1::default(),
                        radius: 1.0,
                    },
                    priority: 9,
                },
            },
        ];
        trim_visibility_candidates(&mut candidates, 2);
        assert_eq!(candidates.len(), 2);
        assert_eq!(candidates[0].candidate.subject_id, 80);
        assert_eq!(candidates[1].candidate.subject_id, 90);
    }

    #[test]
    fn sharp_camera_turn_invalidates_history() {
        assert!(visibility_camera_cut(
            Some([0.0, 0.0, 0.0]),
            Some([0.0, 0.0, -1.0]),
            [0.0, 0.0, 0.0],
            [1.0, 0.0, 0.0],
        ));
        assert!(!visibility_camera_cut(
            Some([0.0, 0.0, 0.0]),
            Some([0.0, 0.0, -1.0]),
            [0.1, 0.0, 0.0],
            [0.0, 0.0, -1.0],
        ));
    }
}

use newengine_camera::Frustum;
use newengine_math::{Mat4, Vec3};
use std::sync::OnceLock;

/// Runtime draw budgets keep the current non-instanced backend path stable.
/// They are intentionally deterministic: nearest objects win, ties are stable-key ordered.
pub(super) const RUNTIME_OPAQUE_PRIMITIVE_BUDGET: usize = 4096;
pub(super) const RUNTIME_SHADOW_PRIMITIVE_BUDGET: usize = 48;
pub(super) const EDITOR_OPAQUE_PRIMITIVE_BUDGET: usize = 4096;
pub(super) const EDITOR_SHADOW_PRIMITIVE_BUDGET: usize = 160;
pub(super) const RUNTIME_FOLIAGE_INSTANCE_BUDGET: usize = 16 * 1024;
pub(super) const EDITOR_FOLIAGE_INSTANCE_BUDGET: usize = 16 * 1024;
pub(super) const RUNTIME_TERRAIN_FORWARD_BUDGET: usize = 64;
pub(super) const RUNTIME_TERRAIN_SHADOW_BUDGET: usize = 64;
pub(super) const EDITOR_TERRAIN_FORWARD_BUDGET: usize = 64;
pub(super) const EDITOR_TERRAIN_SHADOW_BUDGET: usize = 64;
const DEFAULT_SCENE_CULLING_ENABLED: bool = false;

#[derive(Clone, Copy, Debug)]
struct MeshRuntimePolicy {
    primitive_budgets: [[usize; 2]; 2],
    foliage_budgets: [[usize; 2]; 2],
    terrain_budgets: [[usize; 2]; 2],
    terrain_receive_shadows_override: Option<bool>,
    scene_culling_enabled: bool,
    terrain_render_distance: f32,
    primitive_render_distance: [f32; 2],
    primitive_shadow_distance: [f32; 2],
    terrain_near_accept_override: Option<f32>,
    primitive_near_accept_distance: f32,
}

impl MeshRuntimePolicy {
    fn from_process_config() -> Self {
        let usize_var = |name: &str, default: usize, min: usize, max: usize| {
            newengine_runtime_env::var_u64(name, default as u64, min as u64, max as u64) as usize
        };
        let optional_bool = |name: &str| {
            newengine_runtime_env::var(name).map(|value| {
                matches!(
                    value.trim().to_ascii_lowercase().as_str(),
                    "1" | "true" | "yes" | "on"
                )
            })
        };
        let optional_f32 = |name: &str, min: f32, max: f32| {
            newengine_runtime_env::var(name)
                .and_then(|value| value.trim().parse::<f32>().ok())
                .map(|value| value.clamp(min, max))
        };
        let lod_distance_scale = newengine_runtime_env::var_f32(
            newengine_core::startup_window::ENV_LOD_DISTANCE_SCALE,
            1.0,
            0.5,
            2.0,
        );
        let view_distance_meters = newengine_runtime_env::var_f32(
            newengine_core::startup_window::ENV_VIEW_DISTANCE_METERS,
            1000.0,
            100.0,
            10_000.0,
        );
        let lod_scaled =
            |value: f32, min: f32, max: f32| (value * lod_distance_scale).clamp(min, max);

        Self {
            primitive_budgets: [
                [
                    usize_var(
                        "NEWENGINE_EDITOR_OPAQUE_PRIMITIVE_BUDGET",
                        EDITOR_OPAQUE_PRIMITIVE_BUDGET,
                        8,
                        16 * 1024,
                    ),
                    usize_var(
                        "NEWENGINE_EDITOR_SHADOW_PRIMITIVE_BUDGET",
                        EDITOR_SHADOW_PRIMITIVE_BUDGET,
                        8,
                        512,
                    ),
                ],
                [
                    usize_var(
                        "NEWENGINE_RUNTIME_OPAQUE_PRIMITIVE_BUDGET",
                        RUNTIME_OPAQUE_PRIMITIVE_BUDGET,
                        8,
                        16 * 1024,
                    ),
                    usize_var(
                        "NEWENGINE_RUNTIME_SHADOW_PRIMITIVE_BUDGET",
                        RUNTIME_SHADOW_PRIMITIVE_BUDGET,
                        8,
                        512,
                    ),
                ],
            ],
            foliage_budgets: [
                [
                    usize_var(
                        "NEWENGINE_EDITOR_FOLIAGE_INSTANCE_BUDGET",
                        EDITOR_FOLIAGE_INSTANCE_BUDGET,
                        256,
                        64 * 1024,
                    ),
                    usize_var(
                        "NEWENGINE_EDITOR_SHADOW_FOLIAGE_INSTANCE_BUDGET",
                        EDITOR_FOLIAGE_INSTANCE_BUDGET,
                        256,
                        64 * 1024,
                    ),
                ],
                [
                    usize_var(
                        "NEWENGINE_RUNTIME_FOLIAGE_INSTANCE_BUDGET",
                        RUNTIME_FOLIAGE_INSTANCE_BUDGET,
                        256,
                        64 * 1024,
                    ),
                    usize_var(
                        "NEWENGINE_RUNTIME_SHADOW_FOLIAGE_INSTANCE_BUDGET",
                        RUNTIME_FOLIAGE_INSTANCE_BUDGET,
                        256,
                        64 * 1024,
                    ),
                ],
            ],
            terrain_budgets: [
                [
                    usize_var(
                        "NEWENGINE_EDITOR_TERRAIN_FORWARD_BUDGET",
                        EDITOR_TERRAIN_FORWARD_BUDGET,
                        0,
                        256,
                    ),
                    usize_var(
                        "NEWENGINE_EDITOR_TERRAIN_SHADOW_BUDGET",
                        EDITOR_TERRAIN_SHADOW_BUDGET,
                        0,
                        256,
                    ),
                ],
                [
                    usize_var(
                        "NEWENGINE_RUNTIME_TERRAIN_FORWARD_BUDGET",
                        RUNTIME_TERRAIN_FORWARD_BUDGET,
                        0,
                        256,
                    ),
                    usize_var(
                        "NEWENGINE_RUNTIME_TERRAIN_SHADOW_BUDGET",
                        RUNTIME_TERRAIN_SHADOW_BUDGET,
                        0,
                        256,
                    ),
                ],
            ],
            terrain_receive_shadows_override: optional_bool("NEWENGINE_TERRAIN_RECEIVE_SHADOWS"),
            // CPU extraction culling is opt-in. Backend clip-space rejection is authoritative;
            // default-on CPU rejection can hide authored world geometry when bounds are stale
            // or conservative for a particular camera pose.
            scene_culling_enabled: newengine_runtime_env::var_bool(
                "NEWENGINE_RENDER_SCENE_CULLING",
                DEFAULT_SCENE_CULLING_ENABLED,
            ),
            terrain_render_distance: lod_scaled(
                newengine_runtime_env::var_f32(
                    "NEWENGINE_TERRAIN_RENDER_DISTANCE",
                    view_distance_meters,
                    32.0,
                    10_000.0,
                ),
                16.0,
                10_000.0,
            ),
            primitive_render_distance: [
                lod_scaled(
                    newengine_runtime_env::var_f32(
                        "NEWENGINE_PRIMITIVE_RENDER_DISTANCE",
                        view_distance_meters,
                        8.0,
                        10_000.0,
                    ),
                    4.0,
                    10_000.0,
                ),
                lod_scaled(
                    newengine_runtime_env::var_f32(
                        "NEWENGINE_PRIMITIVE_RENDER_DISTANCE",
                        view_distance_meters,
                        8.0,
                        10_000.0,
                    ),
                    4.0,
                    10_000.0,
                ),
            ],
            primitive_shadow_distance: [
                lod_scaled(
                    newengine_runtime_env::var_f32(
                        "NEWENGINE_PRIMITIVE_SHADOW_DISTANCE",
                        240.0,
                        16.0,
                        4096.0,
                    ),
                    8.0,
                    4096.0,
                ),
                lod_scaled(
                    newengine_runtime_env::var_f32(
                        "NEWENGINE_PRIMITIVE_SHADOW_DISTANCE",
                        80.0,
                        16.0,
                        4096.0,
                    ),
                    8.0,
                    4096.0,
                ),
            ],
            terrain_near_accept_override: optional_f32(
                "NEWENGINE_TERRAIN_NEAR_ACCEPT_DISTANCE",
                8.0,
                2048.0,
            ),
            primitive_near_accept_distance: newengine_runtime_env::var_f32(
                "NEWENGINE_PRIMITIVE_NEAR_ACCEPT_DISTANCE",
                12.0,
                1.0,
                512.0,
            ),
        }
    }
}

#[inline]
#[cfg(test)]
fn scale_lod_distance(value: f32, scale: f32, min: f32, max: f32) -> f32 {
    (value * scale.clamp(0.5, 2.0)).clamp(min, max)
}

#[inline]
fn mesh_runtime_policy() -> &'static MeshRuntimePolicy {
    static POLICY: OnceLock<MeshRuntimePolicy> = OnceLock::new();
    POLICY.get_or_init(MeshRuntimePolicy::from_process_config)
}

#[inline]
pub(super) fn translation_of(model: Mat4) -> Vec3 {
    Vec3::new(model.w_axis.x, model.w_axis.y, model.w_axis.z)
}

#[inline]
pub(super) fn distance_sq_to_camera(model: Mat4, camera_position: Vec3) -> f32 {
    let delta = translation_of(model) - camera_position;
    delta.length_squared()
}

#[inline]
pub(super) fn primitive_budget(runtime: bool, shadow_pass: bool) -> usize {
    mesh_runtime_policy().primitive_budgets[runtime as usize][shadow_pass as usize]
}

#[inline]
pub(super) fn foliage_instance_budget(runtime: bool, shadow_pass: bool) -> usize {
    mesh_runtime_policy().foliage_budgets[runtime as usize][shadow_pass as usize]
}

#[inline]
pub(super) fn terrain_budget(runtime: bool, shadow_pass: bool) -> usize {
    mesh_runtime_policy().terrain_budgets[runtime as usize][shadow_pass as usize]
}

#[inline]
pub(super) fn terrain_receive_shadows_enabled(
    policy: newengine_model_domain_api::MeshShadowPolicy,
) -> bool {
    let authored = matches!(
        policy,
        newengine_model_domain_api::MeshShadowPolicy::ReceiveOnly
            | newengine_model_domain_api::MeshShadowPolicy::CastAndReceive
            | newengine_model_domain_api::MeshShadowPolicy::ProfileControlled
    );
    mesh_runtime_policy()
        .terrain_receive_shadows_override
        .unwrap_or(authored)
}

#[inline]
pub(super) fn terrain_cast_shadows_enabled(
    policy: newengine_model_domain_api::MeshShadowPolicy,
) -> bool {
    matches!(
        policy,
        newengine_model_domain_api::MeshShadowPolicy::CastOnly
            | newengine_model_domain_api::MeshShadowPolicy::CastAndReceive
            | newengine_model_domain_api::MeshShadowPolicy::ProfileControlled
    )
}

#[inline]
pub(super) fn primitive_cast_shadows_enabled(
    options: &newengine_model_domain_api::MeshRenderOptions,
) -> bool {
    use newengine_model_domain_api::{MeshRenderRole, MeshShadowPolicy};

    if matches!(
        options.role,
        MeshRenderRole::SkyBackground
            | MeshRenderRole::CelestialBillboard
            | MeshRenderRole::WeatherVolume
            | MeshRenderRole::FirstPersonViewModel
            | MeshRenderRole::CollisionProxy
            | MeshRenderRole::EditorGizmo
            | MeshRenderRole::DebugPrimitive
    ) {
        return false;
    }

    matches!(
        options.shadow_policy,
        MeshShadowPolicy::CastOnly
            | MeshShadowPolicy::CastAndReceive
            | MeshShadowPolicy::ProfileControlled
    )
}

pub(super) trait DistanceKeyEntry {
    fn distance_sq(&self) -> f32;
    fn stable_key(&self) -> u64;
}

impl<T> DistanceKeyEntry for (f32, u64, T) {
    #[inline]
    fn distance_sq(&self) -> f32 {
        self.0
    }

    #[inline]
    fn stable_key(&self) -> u64 {
        self.1
    }
}

impl<T0, T1, T2> DistanceKeyEntry for (f32, u64, T0, T1, T2) {
    #[inline]
    fn distance_sq(&self) -> f32 {
        self.0
    }

    #[inline]
    fn stable_key(&self) -> u64 {
        self.1
    }
}

impl<T0, T1, T2, T3> DistanceKeyEntry for (f32, u64, T0, T1, T2, T3) {
    #[inline]
    fn distance_sq(&self) -> f32 {
        self.0
    }

    #[inline]
    fn stable_key(&self) -> u64 {
        self.1
    }
}

impl<T0, T1, T2, T3, T4> DistanceKeyEntry for (f32, u64, T0, T1, T2, T3, T4) {
    #[inline]
    fn distance_sq(&self) -> f32 {
        self.0
    }

    #[inline]
    fn stable_key(&self) -> u64 {
        self.1
    }
}

impl<T0, T1, T2, T3, T4, T5> DistanceKeyEntry for (f32, u64, T0, T1, T2, T3, T4, T5) {
    #[inline]
    fn distance_sq(&self) -> f32 {
        self.0
    }

    #[inline]
    fn stable_key(&self) -> u64 {
        self.1
    }
}

impl<T0, T1, T2, T3, T4, T5, T6> DistanceKeyEntry for (f32, u64, T0, T1, T2, T3, T4, T5, T6) {
    #[inline]
    fn distance_sq(&self) -> f32 {
        self.0
    }

    #[inline]
    fn stable_key(&self) -> u64 {
        self.1
    }
}

#[inline]
fn compare_distance_then_key<T: DistanceKeyEntry>(a: &T, b: &T) -> std::cmp::Ordering {
    a.distance_sq()
        .partial_cmp(&b.distance_sq())
        .unwrap_or(std::cmp::Ordering::Equal)
        .then_with(|| a.stable_key().cmp(&b.stable_key()))
}

#[inline]
pub(super) fn sort_by_distance_then_key<T: DistanceKeyEntry>(items: &mut [T]) {
    items.sort_by(compare_distance_then_key);
}

/// Keep the same deterministic nearest-first admission semantics without sorting
/// candidates that cannot survive the draw budget. Dense worlds commonly have
/// thousands of candidates while shadow/quality budgets admit only a fraction.
#[inline]
pub(super) fn sort_and_truncate_by_distance_then_key<T: DistanceKeyEntry>(
    items: &mut Vec<T>,
    budget: usize,
) {
    if budget == 0 {
        items.clear();
        return;
    }
    if items.len() > budget {
        let _ = items.select_nth_unstable_by(budget, compare_distance_then_key);
        items.truncate(budget);
    }
    sort_by_distance_then_key(items);
}

#[inline]
pub(super) fn max_axis_scale(model: Mat4) -> f32 {
    let sx = model.x_axis.truncate().length();
    let sy = model.y_axis.truncate().length();
    let sz = model.z_axis.truncate().length();
    sx.max(sy).max(sz).max(0.001)
}

#[inline]
pub(in crate::render_controller::module_impl) fn transform_sphere(model: Mat4, local_center: Vec3, local_radius: f32) -> (Vec3, f32) {
    (
        model.transform_point3(local_center),
        local_radius.abs().max(0.001) * max_axis_scale(model),
    )
}

/// Conservative projected coverage hint used only for streaming priority.
///
/// This is intentionally projection-agnostic: `(radius / distance)^2` preserves the ordering that
/// matters to residency without coupling the asset scheduler to a specific FOV or viewport size.
#[inline]
pub(in crate::render_controller::module_impl) fn sphere_screen_coverage_hint(radius_ws: f32, distance_m: f32) -> f32 {
    let radius = radius_ws.abs().max(0.001);
    if !distance_m.is_finite() || distance_m <= 0.001 {
        return 1.0;
    }
    let angular_ratio = (radius / distance_m).clamp(0.0, 1.0);
    (angular_ratio * angular_ratio).clamp(0.0, 1.0)
}

#[inline]
pub(super) fn shadow_caster_visible(
    cull: Option<super::super::shadows::ShadowCasterCull>,
    center_ws: Vec3,
    radius_ws: f32,
) -> bool {
    cull.map(|c| c.contains_sphere(center_ws, radius_ws))
        .unwrap_or(true)
}

#[derive(Clone, Copy, Debug)]
pub(super) struct PrimitiveVisibilitySettings {
    pub(super) culling_enabled: bool,
    pub(super) frustum: Frustum,
    pub(super) max_distance: f32,
    pub(super) near_accept_distance: f32,
}

#[inline]
pub(super) fn primitive_visibility_settings(
    runtime: bool,
    viewproj: Mat4,
) -> PrimitiveVisibilitySettings {
    PrimitiveVisibilitySettings {
        culling_enabled: render_scene_culling_enabled(),
        frustum: Frustum::from_view_proj(viewproj),
        max_distance: primitive_forward_max_distance(runtime),
        near_accept_distance: primitive_near_accept_distance(),
    }
}

#[inline]
pub(super) fn render_scene_culling_enabled() -> bool {
    // Do not hide world objects on the CPU extraction path by default. The
    // renderer/backend owns actual frustum clipping; the streaming system owns
    // residency. A cheap forward-cone cull is useful as an opt-in stress knob,
    // but as a default it causes visible pop/disappearance while the camera
    // turns, which is not acceptable for gameplay/world presentation.
    mesh_runtime_policy().scene_culling_enabled
}

/// Conservative CPU visibility test used before draw-list materialization.
///
/// The view frustum is extracted once per render pass and reused for every entity. Nearby
/// objects retain a tiny unconditional ring to avoid near-plane churn while everything else
/// must intersect the exact Vulkan/D3D 0..1 clip-space frustum.
#[inline]
pub(in crate::render_controller::module_impl) fn sphere_within_render_distance(
    camera_position: Vec3,
    center_ws: Vec3,
    radius_ws: f32,
    max_distance: f32,
) -> bool {
    let radius = radius_ws.abs().max(0.001);
    let max_d = max_distance.max(radius);
    (center_ws - camera_position).length_squared() <= (max_d + radius) * (max_d + radius)
}

#[inline]
pub(super) fn frustum_sphere_visible(
    frustum: &Frustum,
    camera_position: Vec3,
    center_ws: Vec3,
    radius_ws: f32,
    max_distance: f32,
    near_accept_distance: f32,
) -> bool {
    let radius = radius_ws.abs().max(0.001);
    let max_d = max_distance.max(near_accept_distance).max(radius);
    if !sphere_within_render_distance(camera_position, center_ws, radius, max_d) {
        return false;
    }

    let delta = center_ws - camera_position;
    let dist2 = delta.length_squared();
    let near = near_accept_distance.max(radius * 1.15).max(0.001);
    if dist2 <= near * near {
        return true;
    }
    frustum.contains_sphere(center_ws, radius)
}

#[inline]
pub(super) fn terrain_forward_max_distance() -> f32 {
    mesh_runtime_policy().terrain_render_distance
}

#[inline]
pub(in crate::render_controller::module_impl) fn primitive_forward_max_distance(runtime: bool) -> f32 {
    mesh_runtime_policy().primitive_render_distance[runtime as usize]
}

#[inline]
pub(super) fn primitive_shadow_max_distance(runtime: bool) -> f32 {
    mesh_runtime_policy().primitive_shadow_distance[runtime as usize]
}

#[inline]
pub(super) fn terrain_near_accept_distance(radius_ws: f32) -> f32 {
    mesh_runtime_policy()
        .terrain_near_accept_override
        .unwrap_or_else(|| (radius_ws.abs().max(1.0) * 1.20).clamp(8.0, 2048.0))
}

#[inline]
pub(super) fn primitive_near_accept_distance() -> f32 {
    mesh_runtime_policy().primitive_near_accept_distance
}

#[cfg(test)]
mod startup_lod_scale_tests {
    use super::scale_lod_distance;

    #[test]
    fn cpu_scene_culling_is_opt_in_by_default() {
        const { assert!(!super::DEFAULT_SCENE_CULLING_ENABLED) };
    }

    #[test]
    fn runtime_opaque_budget_covers_dense_authored_worlds() {
        assert!(super::RUNTIME_OPAQUE_PRIMITIVE_BUDGET >= 4096);
    }

    #[test]
    fn render_distance_rejects_far_spheres_without_camera_angle_dependency() {
        let camera = newengine_math::Vec3::ZERO;
        assert!(super::sphere_within_render_distance(
            camera,
            newengine_math::Vec3::new(0.0, 0.0, 99.0),
            2.0,
            100.0,
        ));
        assert!(!super::sphere_within_render_distance(
            camera,
            newengine_math::Vec3::new(0.0, 0.0, 110.0),
            2.0,
            100.0,
        ));
    }

    #[test]
    fn frustum_culler_rejects_far_offscreen_spheres() {
        let frustum = newengine_camera::Frustum::from_view_proj(newengine_math::Mat4::IDENTITY);
        assert!(super::frustum_sphere_visible(
            &frustum,
            newengine_math::Vec3::ZERO,
            newengine_math::Vec3::new(0.0, 0.0, 0.5),
            0.1,
            100.0,
            0.01,
        ));
        assert!(!super::frustum_sphere_visible(
            &frustum,
            newengine_math::Vec3::ZERO,
            newengine_math::Vec3::new(10.0, 0.0, 0.5),
            0.1,
            100.0,
            0.01,
        ));
    }

    #[test]
    fn lod_distance_scale_preserves_and_scales_runtime_ranges() {
        assert_eq!(scale_lod_distance(100.0, 1.0, 8.0, 4096.0), 100.0);
        assert_eq!(scale_lod_distance(100.0, 0.75, 8.0, 4096.0), 75.0);
        assert_eq!(scale_lod_distance(100.0, 1.5, 8.0, 4096.0), 150.0);
        assert_eq!(scale_lod_distance(3000.0, 2.0, 8.0, 4096.0), 4096.0);
    }

    #[test]
    fn budgeted_distance_selection_matches_full_sort_prefix() {
        let source = vec![
            (9.0_f32, 90_u64, "nine"),
            (1.0, 11, "one-b"),
            (4.0, 40, "four"),
            (1.0, 10, "one-a"),
            (16.0, 160, "sixteen"),
            (2.0, 20, "two"),
        ];
        let mut expected = source.clone();
        super::sort_by_distance_then_key(&mut expected);
        expected.truncate(3);

        let mut actual = source;
        super::sort_and_truncate_by_distance_then_key(&mut actual, 3);
        assert_eq!(actual, expected);
    }

    #[test]
    fn budgeted_distance_selection_handles_zero_budget() {
        let mut items = vec![(1.0_f32, 1_u64, ())];
        super::sort_and_truncate_by_distance_then_key(&mut items, 0);
        assert!(items.is_empty());
    }

    #[test]
    fn projected_coverage_hint_prefers_large_near_spheres() {
        let near = super::sphere_screen_coverage_hint(2.0, 4.0);
        let far = super::sphere_screen_coverage_hint(2.0, 40.0);
        let small = super::sphere_screen_coverage_hint(0.25, 4.0);
        assert!(near > far);
        assert!(near > small);
        assert_eq!(super::sphere_screen_coverage_hint(10.0, 0.0), 1.0);
    }
}

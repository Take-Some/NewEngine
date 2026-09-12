#![forbid(unsafe_op_in_unsafe_fn)]

use newengine_assets::{AssetResult, RuntimeTextureAsset};
use newengine_camera_contracts::CameraFrameSnapshot;
use newengine_core::host_events::CursorState;
use newengine_core::render::{Extent2D, RenderHardwareTier, RenderTargetId, SamplerId, TextureId};
use newengine_core::TaskTicket;
use newengine_math::collections::FxHashMap;
use parking_lot::Mutex;
use std::sync::Arc;

use newengine_plugin_manager_bridge::PluginManagerBridge;
use newengine_scene_bridge_runtime::scene_bridge::SceneBridge;
use newengine_viewport_bridge::ViewportBridge;

use super::gpu::{
    DebugLineGpu, GeometryArena, HairGpuRenderer, LitPipeline, MaterialGpuPipeline, MaterialGpuPipelineKey,
    MaterialGpuRegistry, MaterialPipelineBuildProfile, PlayerSkinGpu, PrimitiveGpu, SkinPaletteGpu,
    VfxGpuRenderer,
};
use super::material_bindings::MaterialTextureGpuResidency;
use super::material_plan_cache::ResolvedLitMaterialPlanCache;
use super::metrics::RuntimeOverlayMetrics;
use super::module_impl::draw_lists::RenderDrawListProviderRegistry;
use super::module_impl::gpu_gbuffer_indirect::GpuDrivenGbufferState;
use super::module_impl::gpu_shadow_indirect::GpuShadowIndirectState;
use super::module_impl::gpu_indirect_stream::GpuIndirectStreamBuilder;
use super::module_impl::gpu_scene_table_upload::{GpuSceneTableBuffers, GpuSceneTableUploader};
use super::module_impl::gpu_scene_tables::GpuSceneTables;
use super::module_impl::instancing::InstanceBufferUploader;
use super::module_impl::light_extraction::LightExtractionProviderRegistry;
use super::resource_lifetime::RenderGpuLifetimeQueue;
use super::runtime_profile::RenderRuntimeProfile;

type PrimGpuCache = FxHashMap<newengine_primitives::PrimitiveId, PrimitiveGpu>;
type SkinVertexGpuCache = FxHashMap<newengine_primitives::PrimitiveId, PlayerSkinGpu>;
type SkinPaletteGpuCache = FxHashMap<(u64, u8), SkinPaletteGpu>;
type TerrainGpuCache = FxHashMap<u64, PrimitiveGpu>;

#[derive(Clone, Copy)]
pub struct PerDrawUbo {
    pub ubo: newengine_core::render::BufferId,
    pub bg: newengine_core::render::BindGroupId,
    pub base_texture: TextureId,
    pub normal_texture: TextureId,
    pub roughness_texture: TextureId,
    pub shadow_texture: TextureId,
    pub local_shadow_texture: TextureId,
    pub sampler: SamplerId,
    pub last_seen_frame: u64,
}

/// Profile-owned render feature providers registered by the app/runtime profile.
///
/// The reusable engine render controller starts with empty registries. Application profiles,
/// editor preview, tests or future content plugins must explicitly register the
/// draw-list and light extraction providers they need.
pub(super) struct RenderFeatureProviderState {
    pub(super) draw_list_providers: RenderDrawListProviderRegistry,
    pub(super) light_extraction_providers: LightExtractionProviderRegistry,
}

impl RenderFeatureProviderState {
    #[inline]
    pub(super) fn new() -> Self {
        Self {
            draw_list_providers: RenderDrawListProviderRegistry::new(),
            light_extraction_providers: LightExtractionProviderRegistry::new(),
        }
    }
}

/// Stable bridge references shared by render runtime subsystems.
///
/// This keeps host/plugin/scene access outside the GPU backend adapter and makes
/// the controller a composition root instead of a god-object.
pub(super) struct RenderBridgeState {
    pub(super) viewport: Arc<ViewportBridge>,
    pub(super) plugins: Arc<PluginManagerBridge>,
    pub(super) scene: Arc<SceneBridge>,
}

impl RenderBridgeState {
    #[inline]
    pub(super) fn new(
        viewport: Arc<ViewportBridge>,
        plugins: Arc<PluginManagerBridge>,
        scene: Arc<SceneBridge>,
    ) -> Self {
        Self {
            viewport,
            plugins,
            scene,
        }
    }
}

/// Window/surface/viewport state owned by the engine-facing render runtime.
pub(super) struct RenderViewportState {
    pub(super) clear_color: [f32; 4],
    pub(super) last_w: u32,
    pub(super) last_h: u32,
    /// True while the native window has no presentable surface extent (typically minimized).
    /// The last valid dimensions remain intact so restore can force a backend resize.
    pub(super) surface_suspended: bool,
    pub(super) last_vp_w: u32,
    pub(super) last_vp_h: u32,
    pub(super) last_aspect: f32,
    pub(super) pass_disabled: bool,
    pub(super) render_target: Option<RenderTargetId>,
    pub(super) render_target_extent: Extent2D,
    pub(super) last_cursor_state: CursorState,
}

/// Runtime profile state resolved from declarative host/plugin config.
pub(super) struct RenderRuntimeProfileState {
    pub(super) profile: RenderRuntimeProfile,
    applied_hardware_tier: Option<RenderHardwareTier>,
}

impl RenderRuntimeProfileState {
    #[inline]
    pub(super) fn new() -> Self {
        Self {
            profile: RenderRuntimeProfile::load(),
            applied_hardware_tier: None,
        }
    }

    #[inline]
    pub(super) fn hardware_tier(&self) -> RenderHardwareTier {
        self.applied_hardware_tier
            .unwrap_or(RenderHardwareTier::Unknown)
    }

    pub(super) fn apply_hardware_tier_once(&mut self, tier: RenderHardwareTier) {
        if self.applied_hardware_tier == Some(tier) || tier == RenderHardwareTier::Unknown {
            return;
        }
        if !self.profile.accepts_hardware_tier_resolution() {
            newengine_ulog_api::ulog::info!(
                "render runtime profile: startup profile '{}' is explicit; hardware_tier={:?} will not override user-selected config",
                self.profile.id,
                tier,
            );
            self.applied_hardware_tier = Some(tier);
            return;
        }
        self.profile.apply_hardware_tier(tier);
        self.applied_hardware_tier = Some(tier);
        newengine_ulog_api::ulog::info!(
            "render runtime profile: resolved hardware_tier={:?} effective_profile='{}' gpu_safe={} shadows={} hdr={} postfx={} deferred={} terrain_streaming={}",
            tier,
            self.profile.id,
            self.profile.gpu_safe_enabled(),
            self.profile.shadows_enabled(),
            self.profile.hdr_scene_enabled(),
            self.profile.postfx_enabled(),
            self.profile.deferred_enabled(),
            self.profile.use_runtime_terrain_streaming(),
        );
    }
}

impl RenderViewportState {
    #[inline]
    pub(super) fn new() -> Self {
        Self {
            clear_color: [0.0, 0.0, 0.0, 1.0],
            last_w: 0,
            last_h: 0,
            surface_suspended: false,
            last_vp_w: 0,
            last_vp_h: 0,
            last_aspect: 1.0,
            pass_disabled: false,
            render_target: None,
            render_target_extent: Extent2D::new(0, 0),
            last_cursor_state: CursorState::released(),
        }
    }
}

/// Shadow cache and refresh state. Kept separate from graph/backend adapter state
/// so future shadow providers can be replaced without touching backend.
pub(super) struct RenderShadowRuntimeState {
    pub(super) render_target: Option<RenderTargetId>,
    pub(super) render_target_resolution: u32,
    pub(super) render_target_tile_resolution: u32,
    pub(super) render_target_requested_resolution: u32,
    pub(super) render_target_cascade_count: u32,
    /// Negotiated backend 2D texture extent. CSM tiles must fit the complete atlas
    /// inside this limit; otherwise viewport/scissor and allocation disagree.
    pub(super) max_texture_dimension_2d: u32,
    pub(super) local_render_target: Option<RenderTargetId>,
    pub(super) local_render_target_extent_key: u32,
    pub(super) cache_valid: bool,
    pub(super) local_cache_valid: bool,
    pub(super) warmup_defer_frames_remaining: u8,
    pub(super) caster_observed_tick: u64,
    pub(super) caster_membership_hash: u64,
    /// Cached authoritative caster membership. Rebuilt only when ECS membership/material/visibility
    /// change ticks say topology may have changed; per-frame pose checks iterate this bounded list.
    pub(super) caster_entities: Vec<newengine_ecs::EntityId>,
    pub(super) caster_pose_hash: u64,
    /// Animated skin palettes are render-cadence shadow geometry. Tracking their
    /// revision prevents a cached atlas from holding an old skeletal silhouette
    /// until an unrelated transform/light invalidation happens.
    pub(super) caster_skin_pose_hash: u64,
    pub(super) caster_revision: u64,
    pub(super) cached_caster_revision: u64,
    pub(super) cache_reuse_count: u64,
    pub(super) cache_cold_refresh_count: u64,
    pub(super) cache_projection_refresh_count: u64,
    pub(super) cache_projection_texture_refresh_count: u64,
    pub(super) cache_projection_matrix_refresh_count: u64,
    pub(super) cache_projection_split_refresh_count: u64,
    pub(super) cache_projection_params_refresh_count: u64,
    pub(super) cache_projection_extra_refresh_count: u64,
    pub(super) cache_caster_refresh_count: u64,
    pub(super) caster_entity_change_count: u64,
    pub(super) caster_bounds_change_count: u64,
    pub(super) caster_geometry_change_count: u64,
    pub(super) caster_material_change_count: u64,
    pub(super) caster_visibility_change_count: u64,
    pub(super) current_caster_cull: Option<super::module_impl::shadows::ShadowCasterCull>,
    pub(super) cached_shadow_frame: Option<newengine_render_feature_api::ShadowFrame>,
    pub(super) local_cached_shadow_frame: Option<newengine_render_feature_api::LocalShadowFrame>,
    pub(super) local_cached_caster_revision: u64,
    pub(super) local_cache_reuse_count: u64,
    pub(super) local_cache_refresh_count: u64,
    pub(super) unsupported_point_warning_emitted: bool,
    pub(super) unsupported_spot_warning_emitted: bool,
}

impl RenderShadowRuntimeState {
    #[inline]
    pub(super) fn new() -> Self {
        Self {
            render_target: None,
            render_target_resolution: 0,
            render_target_tile_resolution: 0,
            render_target_requested_resolution: 0,
            render_target_cascade_count: 0,
            max_texture_dimension_2d: newengine_render_api::RenderLimits::default()
                .max_texture_dimension_2d,
            local_render_target: None,
            local_render_target_extent_key: 0,
            cache_valid: false,
            local_cache_valid: false,
            warmup_defer_frames_remaining: super::render_quality::SHADOW_WARMUP_DEFER_FRAMES,
            caster_observed_tick: 0,
            caster_membership_hash: 0,
            caster_entities: Vec::new(),
            caster_pose_hash: 0,
            caster_skin_pose_hash: 0,
            caster_revision: 0,
            cached_caster_revision: 0,
            cache_reuse_count: 0,
            cache_cold_refresh_count: 0,
            cache_projection_refresh_count: 0,
            cache_projection_texture_refresh_count: 0,
            cache_projection_matrix_refresh_count: 0,
            cache_projection_split_refresh_count: 0,
            cache_projection_params_refresh_count: 0,
            cache_projection_extra_refresh_count: 0,
            cache_caster_refresh_count: 0,
            caster_entity_change_count: 0,
            caster_bounds_change_count: 0,
            caster_geometry_change_count: 0,
            caster_material_change_count: 0,
            caster_visibility_change_count: 0,
            current_caster_cull: None,
            cached_shadow_frame: None,
            local_cached_shadow_frame: None,
            local_cached_caster_revision: 0,
            local_cache_reuse_count: 0,
            local_cache_refresh_count: 0,
            unsupported_point_warning_emitted: false,
            unsupported_spot_warning_emitted: false,
        }
    }
}

/// Engine.jobs-backed CPU decode job for a material texture request.
///
/// The render controller owns only the ticket/result bridge. The actual heavy
/// work runs on the engine-runtime job system, not on ad-hoc per-request threads.
pub(super) struct MaterialTextureDecodeJob {
    pub(super) ticket: TaskTicket,
    pub(super) result: Arc<Mutex<Option<AssetResult<RuntimeTextureAsset>>>>,
}

impl MaterialTextureDecodeJob {
    #[inline]
    pub(super) fn is_complete(&self) -> bool {
        self.ticket.is_complete()
    }

    #[inline]
    pub(super) fn take_result(&self) -> Option<AssetResult<RuntimeTextureAsset>> {
        self.result.lock().take()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(super) enum MaterialTextureStreamingClass {
    Secondary = 0,
    StreamingCritical = 1,
    LaunchCritical = 2,
}

#[derive(Clone, Copy, Debug)]
pub(super) struct MaterialTexturePriority {
    pub(super) class: MaterialTextureStreamingClass,
    pub(super) visible_now: bool,
    /// Quantized projected screen coverage. Callers without projection data leave this at zero.
    pub(super) screen_coverage_q: u16,
    /// Closeness score; larger is nearer/more urgent.
    pub(super) proximity_q: u16,
    pub(super) material_importance: u8,
    pub(super) player_weapon_relevance: u8,
    pub(super) mip_urgency: u8,
}

impl MaterialTexturePriority {
    #[inline]
    pub(super) const fn secondary() -> Self {
        Self {
            class: MaterialTextureStreamingClass::Secondary,
            visible_now: false,
            screen_coverage_q: 0,
            proximity_q: 0,
            material_importance: 0,
            player_weapon_relevance: 0,
            mip_urgency: 0,
        }
    }

    #[inline]
    pub(super) const fn streaming_visible() -> Self {
        Self {
            class: MaterialTextureStreamingClass::StreamingCritical,
            visible_now: true,
            screen_coverage_q: 0,
            proximity_q: 0,
            material_importance: 128,
            player_weapon_relevance: 0,
            mip_urgency: 128,
        }
    }

    #[inline]
    pub(super) const fn launch_world() -> Self {
        Self {
            class: MaterialTextureStreamingClass::LaunchCritical,
            visible_now: true,
            screen_coverage_q: u16::MAX,
            proximity_q: u16::MAX,
            material_importance: u8::MAX,
            player_weapon_relevance: 0,
            mip_urgency: u8::MAX,
        }
    }

    #[inline]
    pub(super) const fn launch_player_weapon() -> Self {
        Self {
            class: MaterialTextureStreamingClass::LaunchCritical,
            visible_now: true,
            screen_coverage_q: u16::MAX / 2,
            proximity_q: u16::MAX,
            material_importance: 224,
            player_weapon_relevance: u8::MAX,
            mip_urgency: 224,
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub(super) struct MaterialTextureQueueEntry {
    pub(super) priority: MaterialTexturePriority,
    pub(super) enqueued_frame: u64,
    pub(super) last_touched_frame: u64,
}

#[derive(Clone, Debug)]
pub(super) struct MaterialTextureUploadCandidate {
    pub(super) asset: RuntimeTextureAsset,
    pub(super) payload_bytes: usize,
    pub(super) priority: MaterialTexturePriority,
    pub(super) decoded_frame: u64,
    pub(super) last_touched_frame: u64,
}

/// Material-domain GPU state owned by the render controller.
///
/// This state is deliberately separate from mesh caches and transient lifetime
/// queues so material providers can evolve without turning the controller state
/// into a god-object again.
pub(super) struct RenderMaterialGpuState {
    pub(super) registry: MaterialGpuRegistry,
    pub(super) primary_lit_pipeline_key: Option<MaterialGpuPipelineKey>,
    pub(super) textures: FxHashMap<String, MaterialTextureGpuResidency>,
    /// Last known semantic/visibility priority survives queue pop/retry cycles.
    pub(super) texture_priorities: FxHashMap<String, MaterialTexturePriority>,
    /// Mutable priority queue keyed by logical texture reference. Entries are rescored on demand,
    /// so visibility/camera changes can reprioritize work that has not started decoding yet.
    pub(super) texture_queue: FxHashMap<String, MaterialTextureQueueEntry>,
    pub(super) texture_decode_jobs: FxHashMap<String, MaterialTextureDecodeJob>,
    /// Decoded CPU packets waiting for bounded GPU admission. Kept separate from decode jobs so
    /// CPU concurrency and GPU upload pressure are independently controllable.
    pub(super) texture_upload_queue: FxHashMap<String, MaterialTextureUploadCandidate>,
    pub(super) per_draw_ubo: FxHashMap<(u64, u8), PerDrawUbo>,
    pub(super) resolved_lit_plans: ResolvedLitMaterialPlanCache,
}

impl RenderMaterialGpuState {
    #[inline]
    pub(super) fn new() -> Self {
        Self {
            registry: MaterialGpuRegistry::default(),
            primary_lit_pipeline_key: None,
            textures: FxHashMap::default(),
            texture_priorities: FxHashMap::default(),
            texture_queue: FxHashMap::default(),
            texture_decode_jobs: FxHashMap::default(),
            texture_upload_queue: FxHashMap::default(),
            per_draw_ubo: FxHashMap::default(),
            resolved_lit_plans: ResolvedLitMaterialPlanCache::default(),
        }
    }
}

type ModelBundleLoadResult =
    Arc<Mutex<Option<Result<newengine_model_domain_api::ModelAssetBundle, String>>>>;

pub(super) struct ModelBundleLoadJob {
    pub(super) ticket: TaskTicket,
    pub(super) result: ModelBundleLoadResult,
}

impl ModelBundleLoadJob {
    #[inline]
    pub(super) fn is_complete(&self) -> bool {
        self.ticket.is_complete()
    }

    #[inline]
    pub(super) fn take_result(
        &self,
    ) -> Option<Result<newengine_model_domain_api::ModelAssetBundle, String>> {
        self.result.lock().take()
    }
}

/// Mesh and debug-geometry GPU caches.
pub(super) struct RenderMeshGpuState {
    pub(super) prim_cache: PrimGpuCache,
    pub(super) skin_vertex_cache: SkinVertexGpuCache,
    pub(super) skin_palette_cache: SkinPaletteGpuCache,
    pub(super) terrain_cache: TerrainGpuCache,
    pub(super) model_bundle_cache:
        FxHashMap<String, Arc<newengine_model_domain_api::ModelAssetBundle>>,
    pub(super) model_bundle_jobs: FxHashMap<String, ModelBundleLoadJob>,
    pub(super) model_bundle_failures: FxHashMap<String, String>,
    pub(super) instance_uploader: InstanceBufferUploader,
    pub(super) collision_lines: Option<DebugLineGpu>,
}

impl RenderMeshGpuState {
    #[inline]
    pub(super) fn new() -> Self {
        Self {
            prim_cache: PrimGpuCache::default(),
            skin_vertex_cache: SkinVertexGpuCache::default(),
            skin_palette_cache: SkinPaletteGpuCache::default(),
            terrain_cache: TerrainGpuCache::default(),
            model_bundle_cache: FxHashMap::default(),
            model_bundle_jobs: FxHashMap::default(),
            model_bundle_failures: FxHashMap::default(),
            instance_uploader: InstanceBufferUploader::default(),
            collision_lines: None,
        }
    }
}

/// Deferred native resource destruction queues.
pub(super) struct RenderGpuLifetimeState {
    pub(super) resources: RenderGpuLifetimeQueue,
}

impl RenderGpuLifetimeState {
    #[inline]
    pub(super) fn new() -> Self {
        Self {
            resources: RenderGpuLifetimeQueue::new(),
        }
    }
}

/// GPU-side render controller state grouped by domain.
///
/// The reusable controller owns orchestration caches only. Renderer-native
/// objects stay behind RenderApi/render.api, while profile-owned feature packs
/// provide material, mesh and light extraction behavior explicitly.
pub(super) struct RenderGpuSceneState {
    pub(super) material: RenderMaterialGpuState,
    pub(super) geometry: GeometryArena,
    pub(super) tables: GpuSceneTables,
    pub(super) table_uploader: GpuSceneTableUploader,
    pub(super) table_buffers: Option<GpuSceneTableBuffers>,
    pub(super) indirect_stream: GpuIndirectStreamBuilder,
    pub(super) indirect_gbuffer: GpuDrivenGbufferState,
    pub(super) indirect_shadow: GpuShadowIndirectState,
    pub(super) meshes: RenderMeshGpuState,
    pub(super) lifetimes: RenderGpuLifetimeState,
    pub(super) hair: HairGpuRenderer,
    pub(super) vfx_particles: VfxGpuRenderer,
}

impl RenderGpuSceneState {
    #[inline]
    pub(super) fn new() -> Self {
        Self {
            material: RenderMaterialGpuState::new(),
            geometry: GeometryArena::new(),
            tables: GpuSceneTables::default(),
            table_uploader: GpuSceneTableUploader::new(),
            table_buffers: None,
            indirect_stream: GpuIndirectStreamBuilder::new(),
            indirect_gbuffer: GpuDrivenGbufferState::new(),
            indirect_shadow: GpuShadowIndirectState::new(),
            meshes: RenderMeshGpuState::new(),
            lifetimes: RenderGpuLifetimeState::new(),
            hair: HairGpuRenderer::new(),
            vfx_particles: VfxGpuRenderer::new(),
        }
    }

    #[inline]
    pub(super) fn material_pipeline_profile_for(
        &self,
        scene_color_format: newengine_core::render::TextureFormat,
        deferred_pipelines: bool,
    ) -> MaterialPipelineBuildProfile {
        MaterialPipelineBuildProfile::new(
            scene_color_format,
            super::render_quality::SHADOW_MAP_COLOR_FORMAT,
        )
        .with_deferred_pipelines(deferred_pipelines)
    }

    pub(super) fn primary_lit_pipeline_key(
        &self,
    ) -> newengine_core::EngineResult<MaterialGpuPipelineKey> {
        self.material.primary_lit_pipeline_key.ok_or_else(|| {
            newengine_core::EngineError::other(
                "render material registry: no primary lit material domain selected",
            )
        })
    }

    #[inline]
    pub(super) fn require_material_pipeline_for(
        &mut self,
        key: MaterialGpuPipelineKey,
        scene_color_format: newengine_core::render::TextureFormat,
        deferred_pipelines: bool,
        r: &mut dyn newengine_core::render::RenderApi,
    ) -> newengine_core::EngineResult<MaterialGpuPipeline> {
        let profile = self.material_pipeline_profile_for(scene_color_format, deferred_pipelines);
        self.material.registry.require_pipeline(key, profile, r)
    }

    pub(super) fn require_primary_lit_pipeline_for(
        &mut self,
        scene_color_format: newengine_core::render::TextureFormat,
        deferred_pipelines: bool,
        r: &mut dyn newengine_core::render::RenderApi,
    ) -> newengine_core::EngineResult<LitPipeline> {
        let key = self.primary_lit_pipeline_key()?;
        self.require_material_pipeline_for(key, scene_color_format, deferred_pipelines, r)?
            .lit()
            .ok_or_else(|| {
                newengine_core::EngineError::other(format!(
                "render material registry: selected material domain is not a lit pipeline key='{}'",
                key.as_str()
            ))
            })
    }
}

#[derive(Clone, Copy, Debug)]
pub(super) struct VisibilityHistoryEntry {
    pub(super) consecutive_occluded: u8,
    pub(super) confirmed_occluded: bool,
    pub(super) confidence: f32,
    pub(super) last_produced_frame: u64,
    pub(super) last_motion_frame: u64,
    pub(super) last_seen_frame: u64,
    pub(super) last_center: [f32; 3],
    pub(super) last_radius: f32,
    pub(super) initialized_bounds: bool,
}

impl Default for VisibilityHistoryEntry {
    fn default() -> Self {
        Self {
            consecutive_occluded: 0,
            confirmed_occluded: false,
            confidence: 0.0,
            last_produced_frame: 0,
            last_motion_frame: 0,
            last_seen_frame: 0,
            last_center: [0.0; 3],
            last_radius: 0.0,
            initialized_bounds: false,
        }
    }
}

pub(super) struct RenderVisibilityRuntimeState {
    pub(super) history: FxHashMap<u64, VisibilityHistoryEntry>,
    pub(super) scene_key: Option<usize>,
    pub(super) last_camera_position: Option<[f32; 3]>,
    pub(super) last_camera_forward: Option<[f32; 3]>,
    pub(super) last_viewport_extent: Option<[u32; 2]>,
    pub(super) last_submission_frame: u64,
    pub(super) last_provider_frame: u64,
    pub(super) last_source_count: usize,
    pub(super) last_eligible_count: usize,
    pub(super) last_candidate_count: usize,
    pub(super) last_result_count: usize,
    pub(super) service_failures: u64,
}

impl RenderVisibilityRuntimeState {
    #[inline]
    pub(super) fn new() -> Self {
        Self {
            history: FxHashMap::default(),
            scene_key: None,
            last_camera_position: None,
            last_camera_forward: None,
            last_viewport_extent: None,
            last_submission_frame: 0,
            last_provider_frame: 0,
            last_source_count: 0,
            last_eligible_count: 0,
            last_candidate_count: 0,
            last_result_count: 0,
            service_failures: 0,
        }
    }

    #[inline]
    pub(super) fn clear_history(&mut self) {
        self.history.clear();
        self.last_provider_frame = 0;
        self.last_source_count = 0;
        self.last_eligible_count = 0;
        self.last_candidate_count = 0;
        self.last_result_count = 0;
    }
}

/// Frame/gameplay view state required by extraction. Renderer backend adapters
/// must not access this directly; it is consumed before RenderFrameEnvelope is
/// submitted.
pub(super) struct RenderFrameRuntimeState {
    pub(super) frame_index: u64,
    pub(super) last_pick_seq: u64,
    /// Selection produced while the render orchestration holds a scene lock.
    /// Applied by the playable viewport after that lock is released.
    pub(super) pending_pick_selection: Option<Option<newengine_ecs::EntityId>>,
    pub(super) pending_pick_additive: bool,
    /// Last camera frame observed by render orchestration.
    ///
    /// This is a pure DTO snapshot from the camera contract boundary. Render
    /// runtime must not own `newengine-camera` projection/controller/nav state.
    pub(super) last_camera_snapshot: Option<CameraFrameSnapshot>,
    /// Delayed engine.visibility control-plane history and hysteresis state.
    pub(super) visibility: RenderVisibilityRuntimeState,
    /// Frame-coherent primitive extraction reused by shadow, GBuffer and forward passes.
    pub(super) primitive_scene_snapshot:
        Option<Arc<super::module_impl::frame_snapshots::PrimitiveSceneSnapshot>>,
    /// CSM skinned caster admission captured once and reused by every directional cascade.
    pub(super) skinned_shadow_scene_snapshot:
        Option<Arc<super::module_impl::frame_snapshots::SkinnedShadowSceneSnapshot>>,
    /// Frame-local GPU/material resolution for skinned casters. Cascades consume immutable handles
    /// and perform only cascade-specific culling, UBO update and draw recording.
    pub(super) prepared_skinned_shadow_plan:
        Option<Arc<super::module_impl::frame_snapshots::PreparedSkinnedShadowFramePlan>>,
    pub(super) sim_schedule: newengine_sim::SimSchedule,
    pub(super) gameplay_systems:
        newengine_gameplay_world_runtime::gameplay::GameplaySystemProviderRegistry,
    pub(super) gameplay_content:
        newengine_gameplay_world_runtime::gameplay::GameplayContentProviderRegistry,
    pub(super) gameplay_ui: newengine_gameplay_world_runtime::gameplay::GameplayUiProviderRegistry,
    pub(super) gameplay_physics_queries:
        newengine_gameplay_world_runtime::gameplay::GameplayPhysicsQueryProviderRegistry,
    pub(super) world_runtime: newengine_world_runtime_api::WorldRuntimeProviderRegistry,
    pub(super) input_systems: newengine_input_systems_runtime::InputRuntimeSystems,
    pub(super) last_play_mode: newengine_gameplay_world_runtime::gameplay::GameRunMode,
}

impl RenderFrameRuntimeState {
    #[inline]
    pub(super) fn new() -> Self {
        Self {
            frame_index: 0,
            last_pick_seq: 0,
            pending_pick_selection: None,
            pending_pick_additive: false,
            last_camera_snapshot: None,
            visibility: RenderVisibilityRuntimeState::new(),
            primitive_scene_snapshot: None,
            skinned_shadow_scene_snapshot: None,
            prepared_skinned_shadow_plan: None,
            sim_schedule: newengine_gameplay_world_runtime::gameplay::default_sim_schedule(),
            gameplay_systems: newengine_gameplay_world_runtime::gameplay::GameplaySystemProviderRegistry::new(),
            gameplay_content: newengine_gameplay_world_runtime::gameplay::GameplayContentProviderRegistry::new(),
            gameplay_ui: newengine_gameplay_world_runtime::gameplay::GameplayUiProviderRegistry::new(),
            gameplay_physics_queries: newengine_gameplay_world_runtime::gameplay::GameplayPhysicsQueryProviderRegistry::new(),
            world_runtime: newengine_world_runtime_api::WorldRuntimeProviderRegistry::new(),
            input_systems: newengine_input_systems_runtime::InputRuntimeSystems::new(),
            last_play_mode: newengine_gameplay_world_runtime::gameplay::GameRunMode::Staging,
        }
    }
}

pub(super) struct RenderDiagnosticsRuntimeState {
    pub(super) overlay_metrics: RuntimeOverlayMetrics,
}

impl RenderDiagnosticsRuntimeState {
    #[inline]
    pub(super) fn new() -> Self {
        Self {
            overlay_metrics: RuntimeOverlayMetrics::new(),
        }
    }
}

pub(super) struct RenderUiSurfaceRuntimeState {
    pub(super) primary: super::module_impl::ui_node_surface::RenderUiNodeSurfaceState,
}

impl RenderUiSurfaceRuntimeState {
    #[inline]
    pub(super) fn new() -> Self {
        Self {
            primary: super::module_impl::ui_node_surface::RenderUiNodeSurfaceState::new(),
        }
    }
}

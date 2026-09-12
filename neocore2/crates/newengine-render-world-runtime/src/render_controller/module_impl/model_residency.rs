#![forbid(unsafe_op_in_unsafe_fn)]

use std::sync::Arc;

use abi_stable::std_types::RString;
use newengine_core::render::RenderApi;
use newengine_core::{EngineResult, TaskLane, TaskPriority, TaskRequest, ThreadPoolHandle};
use newengine_model_domain_api::{
    AssetGraphResolveRequest, ModelAssetBundle, ModelAssetRequest, ResolvedAssetGraphV2,
    ASSET_GRAPH_METHOD_RESOLVE_V1, ENGINE_ASSETS_GRAPH_SERVICE_ID,
};
use newengine_plugin_api::{Blob, MethodName};
use newengine_primitives::{fnv1a_64, PrimitiveId};
use newengine_scene::Scene;
use parking_lot::Mutex;

use crate::render_controller::gpu::upload_primitive_mesh;
use crate::render_controller::state::ModelBundleLoadJob;
use newengine_gameplay_world_runtime::gameplay::ModelRenderComponent;

use super::RuntimeRenderController;

const MODEL_BUNDLE_CACHE_CAPACITY: usize = 32;
const MODEL_FAILURE_CACHE_CAPACITY: usize = 64;

impl RuntimeRenderController {
    #[inline]
    pub(super) fn cached_model_bundle(&self, logical_path: &str) -> Option<Arc<ModelAssetBundle>> {
        self.gpu
            .meshes
            .model_bundle_cache
            .get(logical_path.trim())
            .map(Arc::clone)
    }

    pub(super) fn model_bundle_bounds(
        bundle: &ModelAssetBundle,
    ) -> Option<(newengine_math::Vec3, f32)> {
        let mut min = newengine_math::Vec3::splat(f32::INFINITY);
        let mut max = newengine_math::Vec3::splat(f32::NEG_INFINITY);
        let mut found = false;
        for part in &bundle.parts {
            let radius = part.mesh.bounds_radius.abs();
            if !part.mesh.bounds_center.is_finite() || !radius.is_finite() {
                continue;
            }
            let extent = newengine_math::Vec3::splat(radius);
            min = min.min(part.mesh.bounds_center - extent);
            max = max.max(part.mesh.bounds_center + extent);
            found = true;
        }
        if !found {
            return None;
        }
        let center = (min + max) * 0.5;
        let radius = (max - center).length().max(0.001);
        Some((center, radius))
    }

    #[inline]
    pub(super) fn model_part_primitive_id(
        bundle: &ModelAssetBundle,
        part_index: usize,
    ) -> PrimitiveId {
        let identity = format!(
            "model.runtime:{}:{}:{}",
            bundle.source, bundle.dependency_graph.stable_cache_key, part_index
        );
        PrimitiveId::new(fnv1a_64(&identity))
    }

    /// Advances imported model CPU preparation and GPU mesh residency without
    /// performing graph/model decode work on the render thread.
    ///
    /// Every resident model part is backfilled into the additive geometry arena as part of the
    /// same bounded upload budget. This keeps legacy direct rendering authoritative while making
    /// geometry residency independent from the path that first populated `prim_cache`.
    pub(super) fn pump_model_residency(
        &mut self,
        r: &mut dyn RenderApi,
        scene: &Scene,
        thread_pool: Option<&ThreadPoolHandle>,
    ) -> EngineResult<u32> {
        let active_sources = scene
            .world()
            .query::<ModelRenderComponent>()
            .filter_map(|(_, model)| {
                let source = model.logical_path.trim();
                (!source.is_empty()).then(|| source.to_owned())
            })
            .collect::<std::collections::BTreeSet<_>>();

        self.poll_model_bundle_jobs();
        self.trim_model_bundle_cache(&active_sources);
        self.submit_model_bundle_jobs(&active_sources, thread_pool);

        let upload_budget =
            newengine_runtime_policy::streaming_policy().model_gpu_uploads_per_frame;
        if upload_budget == 0 {
            return Ok(0);
        }

        let mut uploaded = 0u32;
        'sources: for source in &active_sources {
            let Some(bundle) = self.cached_model_bundle(source) else {
                continue;
            };
            for (part_index, part) in bundle.parts.iter().enumerate() {
                if uploaded >= upload_budget {
                    break 'sources;
                }
                let primitive_id = Self::model_part_primitive_id(&bundle, part_index);
                let legacy_ready = self.gpu.meshes.prim_cache.contains_key(&primitive_id);
                let arena_ready = !self.gpu.geometry.is_enabled()
                    || self.gpu.geometry.handle_for(primitive_id).is_some();
                if legacy_ready && arena_ready {
                    continue;
                }

                if !legacy_ready {
                    let label = format!("model.runtime:{}:{}", source, part_index);
                    let gpu = upload_primitive_mesh(r, &part.mesh, &label)?;
                    self.gpu.meshes.prim_cache.insert(primitive_id, gpu);
                }
                if !arena_ready {
                    let _ = self
                        .gpu
                        .geometry
                        .ensure_primitive_fail_open(r, primitive_id, &part.mesh);
                }
                uploaded = uploaded.saturating_add(1);
            }
        }

        if uploaded > 0 {
            self.invalidate_shadow_cache();
            self.invalidate_local_shadow_cache();
        }

        if uploaded > 0 && newengine_ulog_api::ulog::trace_enabled() {
            let geometry = self.gpu.geometry.stats();
            newengine_ulog_api::ulog::trace!(
                "model residency: gpu uploaded/backfilled frame={} parts={} budget={} active_models={} cpu_bundles={} pending_jobs={} geometry_pages={} geometry_resident={} geometry_retired={}",
                self.frame.frame_index,
                uploaded,
                upload_budget,
                active_sources.len(),
                self.gpu.meshes.model_bundle_cache.len(),
                self.gpu.meshes.model_bundle_jobs.len(),
                geometry.pages,
                geometry.resident_slots,
                geometry.retired_slots,
            );
        }
        Ok(uploaded)
    }

    fn poll_model_bundle_jobs(&mut self) {
        let ready = self
            .gpu
            .meshes
            .model_bundle_jobs
            .iter()
            .filter(|(_, job)| job.is_complete())
            .map(|(source, _)| source.clone())
            .collect::<Vec<_>>();

        for source in ready {
            let Some(job) = self.gpu.meshes.model_bundle_jobs.remove(&source) else {
                continue;
            };
            match job.take_result() {
                Some(Ok(bundle)) => {
                    let parts = bundle.parts.len();
                    self.gpu
                        .meshes
                        .model_bundle_cache
                        .insert(source.clone(), Arc::new(bundle));
                    self.gpu.meshes.model_bundle_failures.remove(&source);
                    newengine_ulog_api::ulog::debug!(
                        "model residency: cpu bundle ready path='{}' parts={} frame={} lane='render-prep'",
                        source,
                        parts,
                        self.frame.frame_index,
                    );
                }
                Some(Err(error)) => {
                    self.record_model_bundle_failure(source, error);
                }
                None => {
                    self.record_model_bundle_failure(
                        source,
                        "model render-prep task completed without a result".to_owned(),
                    );
                }
            }
        }
    }

    fn submit_model_bundle_jobs(
        &mut self,
        active_sources: &std::collections::BTreeSet<String>,
        thread_pool: Option<&ThreadPoolHandle>,
    ) {
        let Some(thread_pool) = thread_pool else {
            for source in active_sources {
                if self.gpu.meshes.model_bundle_cache.contains_key(source)
                    || self.gpu.meshes.model_bundle_jobs.contains_key(source)
                    || self.gpu.meshes.model_bundle_failures.contains_key(source)
                {
                    continue;
                }
                self.record_model_bundle_failure(
                    source.clone(),
                    "engine.threading unavailable for model RenderPrep".to_owned(),
                );
            }
            return;
        };

        let configured =
            newengine_runtime_policy::streaming_policy().model_render_prep_jobs as usize;
        let max_jobs = configured
            .min(thread_pool.worker_threads().saturating_sub(1).max(1))
            .max(1);
        let mut free_slots = max_jobs.saturating_sub(self.gpu.meshes.model_bundle_jobs.len());
        if free_slots == 0 {
            return;
        }

        for source in active_sources {
            if free_slots == 0 {
                break;
            }
            if self.gpu.meshes.model_bundle_cache.contains_key(source)
                || self.gpu.meshes.model_bundle_jobs.contains_key(source)
                || self.gpu.meshes.model_bundle_failures.contains_key(source)
            {
                continue;
            }

            let worker_source = source.clone();
            let result = Arc::new(Mutex::new(None));
            let result_out = Arc::clone(&result);
            let request = TaskRequest::new("model.runtime.prepare")
                .with_source("render.controller")
                .with_owner("engine.render")
                .with_category("model-render-prep")
                .with_lane(TaskLane::RenderPrep)
                .with_priority(TaskPriority::Interactive)
                .with_frame_id(self.frame.frame_index)
                .with_dependency_group(format!(
                    "frame.{}.model-render-prep",
                    self.frame.frame_index
                ))
                .with_task_domain(newengine_task_api::task_domain::ENGINE_RENDER_PREP)
                .with_task_pass(newengine_task_api::task_pass::FEATURE_EXTRACT)
                .with_task_id(format!("render.model.prepare.{:016x}", fnv1a_64(source)));
            let ticket = thread_pool.submit_request(request, move || {
                *result_out.lock() = Some(load_model_bundle(&worker_source));
            });
            self.gpu
                .meshes
                .model_bundle_jobs
                .insert(source.clone(), ModelBundleLoadJob { ticket, result });
            free_slots -= 1;
        }
    }

    fn record_model_bundle_failure(&mut self, source: String, error: String) {
        if self.gpu.meshes.model_bundle_failures.len() >= MODEL_FAILURE_CACHE_CAPACITY {
            self.gpu.meshes.model_bundle_failures.clear();
        }
        newengine_ulog_api::ulog::warn!(
            "model residency: render-prep failed path='{}' err='{}'",
            source,
            error,
        );
        self.gpu.meshes.model_bundle_failures.insert(source, error);
    }

    fn trim_model_bundle_cache(&mut self, active_sources: &std::collections::BTreeSet<String>) {
        if self.gpu.meshes.model_bundle_cache.len() <= MODEL_BUNDLE_CACHE_CAPACITY {
            return;
        }
        let inactive = self
            .gpu
            .meshes
            .model_bundle_cache
            .keys()
            .filter(|source| !active_sources.contains(*source))
            .cloned()
            .collect::<Vec<_>>();
        for source in inactive {
            if self.gpu.meshes.model_bundle_cache.len() <= MODEL_BUNDLE_CACHE_CAPACITY {
                break;
            }
            self.gpu.meshes.model_bundle_cache.remove(&source);
        }
    }
}

fn load_model_bundle(logical_path: &str) -> Result<ModelAssetBundle, String> {
    let host = newengine_plugin_host::default_host_api();
    let graph_payload = serde_json::to_vec(&AssetGraphResolveRequest {
        root_ref: logical_path.to_owned(),
    })
    .map_err(|error| error.to_string())?;
    let graph_bytes = (host.call_service_v1)(
        RString::from(ENGINE_ASSETS_GRAPH_SERVICE_ID),
        MethodName::from(ASSET_GRAPH_METHOD_RESOLVE_V1),
        Blob::from(graph_payload),
    )
    .into_result()
    .map(|value| value.into_vec())
    .map_err(|error| error.to_string())?;
    let dependency_graph = serde_json::from_slice::<ResolvedAssetGraphV2>(&graph_bytes)
        .map_err(|error| format!("engine.assets.graph returned invalid graph: {error}"))?;

    let mut request = ModelAssetRequest::new(logical_path.to_owned());
    request.dependency_graph = Some(dependency_graph);
    newengine_model_client::ModelGatewayClient::new(host).assemble_bundle(&request)
}

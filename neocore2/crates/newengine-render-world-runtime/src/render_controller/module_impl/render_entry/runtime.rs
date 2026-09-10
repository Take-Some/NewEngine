use std::time::Instant;

use super::*;

fn trace_backend_phase_graph_dump(frame_index: u64, trace_frame: bool) {
    if !trace_frame || frame_index <= 3 {
        return;
    }

    let response = newengine_plugin_host::call_service_v1(
        newengine_plugin_api::CapabilityId::from(newengine_render_api::ENGINE_RENDER_SERVICE_ID),
        newengine_plugin_api::MethodName::from(
            newengine_render_api::RENDER_SERVICE_METHOD_DUMP_PHASE_GRAPH_V1,
        ),
        newengine_plugin_api::Blob::from(Vec::new()),
    );
    let abi_stable::std_types::RResult::ROk(blob) = response else {
        return;
    };
    let Ok(json) = std::str::from_utf8(blob.as_slice()) else {
        return;
    };
    newengine_ulog_api::ulog::info!(
        "render backend phase graph dump: frame={} json={}",
        frame_index,
        json
    );
}

impl RuntimeRenderController {
    pub(in crate::render_controller::module_impl) fn render_runtime_module<E: Send + 'static>(
        &mut self,
        ctx: &mut ModuleCtx<'_, E>,
    ) -> EngineResult<()> {
        let render_module_started = Instant::now();
        let plugin_snapshot = ctx
            .resources()
            .get::<newengine_plugin_host::PluginsSnapshot>()
            .cloned();
        self.sync_plugin_bridge(ctx, plugin_snapshot.as_ref());

        let (w, h) = Self::read_window_size(ctx);
        let backend_work_budget = ctx
            .resources()
            .get::<newengine_render_runtime_adapter::ResolvedRenderBackendConfig>()
            .map(|cfg| {
                self.viewport.clear_color = cfg.clear_color;
                self.apply_backend_capability_profile(&cfg.capabilities);
                cfg.work_budget
            });

        if w == 0 || h == 0 {
            self.suspend_zero_sized_surface(w, h);
            return Ok(());
        }

        let trace_frame = crate::render_controller::module_impl::trace_policy::should_trace_frame(
            self.frame.frame_index,
        );
        let api = match require_render_api(ctx) {
            Ok(api) => api.clone(),
            Err(_) => return Ok(()),
        };
        if self.backend_render_disabled() {
            ctx.resources_mut().insert(self.backend_status_snapshot());
            ctx.resources_mut().insert(SceneLaunchStatus::inactive());
            return Ok(());
        }
        let mut r = api.lock();
        if let Some(budget) = backend_work_budget {
            let _ = r.set_work_budget(budget);
        }
        self.bridge_render_backend_events(ctx, &mut **r);

        let material_upload_jobs = backend_work_budget
            .map(|b| b.max_upload_jobs_per_frame.max(1))
            .unwrap_or(1);
        let thread_pool = ctx.thread_pool().cloned();
        self.trace_render_begin(trace_frame, w, h);

        let prelaunch_status = match self.handle_prelaunch_gate(
            ctx,
            &mut **r,
            backend_work_budget,
            material_upload_jobs,
            trace_frame,
            w,
            h,
        ) {
            Ok(status) => status,
            Err(error) if is_backend_device_lost_error(&error) => {
                self.record_render_backend_error("render.prelaunch_gate", error)?;
                drop(r);
                ctx.resources_mut().insert(self.backend_status_snapshot());
                ctx.resources_mut().insert(SceneLaunchStatus::inactive());
                return Ok(());
            }
            Err(error) => return Err(error),
        };
        if let Some(status) = prelaunch_status {
            if !status.active {
                if let Some(flow) = ctx.resources_mut().get_mut::<UiPresentationFlowState>() {
                    flow.mark_runtime_ready(
                        self.frame.frame_index,
                        "renderer launch gate released; runtime presentation may activate",
                    );
                }
            }
            drop(r);
            ctx.resources_mut().insert(status);
            return Ok(());
        }

        let fixed_step_count_for_streaming =
            ctx.frame().map(|frame| frame.fixed_step_count).unwrap_or(1);
        let playable_decode_ceiling =
            crate::render_controller::render_quality::material_texture_playable_async_decode_ceiling(
                self.runtime_profile.hardware_tier(),
            ) as u32;
        let decode_start_jobs =
            crate::render_controller::render_quality::material_texture_playable_start_budget(
                self.runtime_profile.hardware_tier(),
                material_upload_jobs,
                fixed_step_count_for_streaming,
            );
        self.pump_material_texture_requests(
            &mut **r,
            thread_pool.as_ref(),
            decode_start_jobs,
            playable_decode_ceiling,
        );

        self.resize_if_needed(&mut **r, w, h)?;
        drop(r);

        let ui_layers = ctx
            .resources_mut()
            .remove::<UiLayerDrawPacketSet>()
            .unwrap_or_else(|| UiLayerDrawPacketSet::new(self.frame.frame_index));
        let primary_ui_domain = ctx
            .resources()
            .get::<UiLayerCompositionPlan>()
            .map(|plan| plan.domain)
            .unwrap_or_else(|| {
                match ctx
                    .resources()
                    .get::<UiScreenProfileState>()
                    .map(|state| state.descriptor.profile)
                    .unwrap_or_default()
                {
                    UiScreenProfile::Game => UiLayerDomain::GameViewport,
                    UiScreenProfile::Headless => UiLayerDomain::System,
                }
            });
        self.apply_editor_viewport_slot(ctx, w, h);
        let mut r = api.lock();
        let (dt, fixed_dt, fixed_alpha, fixed_step_count, fixed_tick) = ctx
            .frame()
            .map(|frame| {
                (
                    frame.dt,
                    frame.fixed_dt,
                    frame.fixed_alpha,
                    frame.fixed_step_count,
                    frame.fixed_tick,
                )
            })
            .unwrap_or((0.016, 0.016, 0.0, 1, 0));
        let pre_begin_ms = render_module_started.elapsed().as_secs_f64() * 1000.0;
        let backend_begin_started = Instant::now();
        let scope_result = self.begin_playable_surface_frame(
            &mut **r,
            !ui_layers.is_empty(),
            w,
            h,
            dt,
            fixed_dt,
            fixed_alpha,
            fixed_step_count,
            fixed_tick,
            trace_frame,
        );
        let backend_begin_ms = backend_begin_started.elapsed().as_secs_f64() * 1000.0;
        let Some(scope) = (match scope_result {
            Ok(scope) => scope,
            Err(e) if is_backend_device_lost_error(&e) => {
                self.record_render_backend_error("render.begin_frame", e)?;
                drop(r);
                ctx.resources_mut().insert(self.backend_status_snapshot());
                ctx.resources_mut().insert(SceneLaunchStatus::inactive());
                return Ok(());
            }
            Err(e) => return Err(e),
        }) else {
            drop(r);
            ctx.resources_mut().insert(SceneLaunchStatus::inactive());
            return Ok(());
        };

        self.frame.frame_index = self.frame.frame_index.saturating_add(1).max(1);
        self.gpu
            .meshes
            .instance_uploader
            .begin_frame(self.frame.frame_index);
        self.diagnostics.overlay_metrics.begin_frame(scope.dt);

        let playable_started = Instant::now();
        let outcome = match catch_unwind(AssertUnwindSafe(|| {
            self.render_playable_viewport_frame(
                ctx,
                &mut **r,
                plugin_snapshot.as_ref(),
                ui_layers,
                primary_ui_domain,
                scope,
            )
        })) {
            Ok(Ok(outcome)) => outcome,
            Ok(Err(e)) => {
                let message = e.to_string();
                if is_backend_device_lost_error(&e) {
                    self.record_render_backend_error("render.playable_frame.error", e)?;
                    drop(r);
                    ctx.resources_mut().insert(self.backend_status_snapshot());
                    ctx.resources_mut().insert(SceneLaunchStatus::inactive());
                    return Ok(());
                }
                if is_transient_shader_pipeline_error(&e) {
                    newengine_ulog_api::ulog::warn!(
                        "render controller: playable frame yielded while shader pipeline is pending; keeping viewport pass retryable: {}",
                        message
                    );
                    let _ = r.discard_recorded_commands();
                    let _ = r.end_frame();
                    drop(r);
                    ctx.resources_mut().insert(SceneLaunchStatus::inactive());
                    return Ok(());
                }
                self.disable_viewport_pass("render_playable_viewport_frame.error", &message);
                newengine_ulog_api::ulog::error!(
                    "render controller: playable frame returned error; presenting degraded recovery frame instead of aborting: {}",
                    message
                );
                let _ = r.discard_recorded_commands();
                let _ = r.end_frame();
                drop(r);
                ctx.resources_mut()
                    .insert(newengine_core::render::RenderBackendStatus::degraded(
                        "render.playable_frame.error",
                        message,
                    ));
                ctx.resources_mut().insert(SceneLaunchStatus::inactive());
                return Ok(());
            }
            Err(payload) => {
                let message = panic_payload_message(payload);
                self.disable_viewport_pass("render_playable_viewport_frame.panic", &message);
                newengine_ulog_api::ulog::error!(
                    "render controller: caught panic during playable frame; presenting degraded recovery frame instead of aborting: {}",
                    message
                );
                newengine_core::crash::record_breadcrumb(format!(
                    "render controller: caught playable-frame panic frame={} msg='{}'",
                    self.frame.frame_index, message
                ));
                let _ = r.discard_recorded_commands();
                let _ = r.end_frame();
                drop(r);
                ctx.resources_mut()
                    .insert(newengine_core::render::RenderBackendStatus::degraded(
                        "render.playable_frame.panic",
                        message,
                    ));
                ctx.resources_mut().insert(SceneLaunchStatus::inactive());
                return Ok(());
            }
        };
        let playable_frame_ms = playable_started.elapsed().as_secs_f64() * 1000.0;

        let mut telemetry_to_publish = None;
        let mut ui_telemetry_to_publish = None;

        let mut frame_debug_snapshot = match outcome {
            PlayableFrameOutcome::Continue {
                frame_debug_snapshot,
            } => frame_debug_snapshot,
            PlayableFrameOutcome::BackendDeferred => {
                // Bounded WSI back-pressure is a normal host scheduling outcome. The backend did
                // not open/submit a native frame, so do not present, degrade backend state, or
                // mutate SceneLaunchStatus. The next redraw rebuilds and retries the frame.
                drop(r);
                return Ok(());
            }
            PlayableFrameOutcome::EndedEarly { ui_telemetry } => {
                ui_telemetry_to_publish = ui_telemetry;
                drop(r);
                if self.backend_render_disabled() {
                    ctx.resources_mut().insert(self.backend_status_snapshot());
                }
                ctx.resources_mut().insert(SceneLaunchStatus::inactive());
                if let Some(ui_telemetry) = ui_telemetry_to_publish {
                    ctx.resources_mut().insert(ui_telemetry);
                }
                return Ok(());
            }
        };

        let diagnostics_before_present_ms;
        let backend_end_ms;
        let backend_reported_begin_ms;
        let backend_frame_slot_wait_ms;
        let backend_surface_acquire_ms;
        let backend_image_wait_ms;
        let backend_reported_end_ms;
        let backend_gpu_timestamps_enabled;
        let backend_gpu_timing_frame_index;
        let backend_gpu_shadow_ms;
        let backend_gpu_opaque_ms;
        let backend_gpu_postfx_ms;
        let backend_gpu_ui_ms;
        let backend_gpu_profiled_ms;
        {
            let diagnostics_before_present_started = Instant::now();
            if let Ok(diag) = r.diagnostics_snapshot() {
                self.diagnostics
                    .overlay_metrics
                    .record_backend_snapshot(&diag);
                if let Some(snapshot) = frame_debug_snapshot.as_mut() {
                    snapshot.queued_upload_jobs = diag.queue.queued_upload_jobs;
                    snapshot.queued_upload_bytes = diag.queue.queued_upload_bytes;
                    snapshot.resource_buffers = diag.resources.buffers;
                    snapshot.resource_textures = diag.resources.textures;
                    snapshot.resource_pipelines = diag.resources.pipelines;
                }
            }

            if trace_frame {
                newengine_core::crash::record_breadcrumb(format!(
                    "render controller: end_frame frame={}",
                    self.frame.frame_index
                ));
            }
            diagnostics_before_present_ms =
                diagnostics_before_present_started.elapsed().as_secs_f64() * 1000.0;
            let backend_end_started = Instant::now();
            if let Err(e) = r.end_frame() {
                if is_backend_device_lost_error(&e) {
                    self.record_render_backend_error("render.end_frame", e)?;
                    drop(r);
                    ctx.resources_mut().insert(self.backend_status_snapshot());
                    ctx.resources_mut().insert(SceneLaunchStatus::inactive());
                    return Ok(());
                }
                return Err(e);
            }
            backend_end_ms = backend_end_started.elapsed().as_secs_f64() * 1000.0;

            // Successful frames are the common path, so resource retirement must be
            // serviced here as well as in degraded/early-exit paths. Collect only
            // after end_frame: backend completion events can now prove which old
            // per-draw buffers, bind groups, and deferred targets are safe to destroy.
            self.gc_per_draw_ubos(&mut **r);
            self.gc_deferred_rts(&mut **r);

            let backend_timing_snapshot = r.diagnostics_snapshot().ok();
            backend_reported_begin_ms = backend_timing_snapshot
                .as_ref()
                .map(|diag| diag.frame.last_begin_frame_ms)
                .unwrap_or(0.0);
            backend_frame_slot_wait_ms = backend_timing_snapshot
                .as_ref()
                .map(|diag| diag.frame.last_frame_slot_wait_ms)
                .unwrap_or(0.0);
            backend_surface_acquire_ms = backend_timing_snapshot
                .as_ref()
                .map(|diag| diag.frame.last_surface_acquire_ms)
                .unwrap_or(0.0);
            backend_image_wait_ms = backend_timing_snapshot
                .as_ref()
                .map(|diag| diag.frame.last_image_wait_ms)
                .unwrap_or(0.0);
            backend_reported_end_ms = backend_timing_snapshot
                .as_ref()
                .map(|diag| diag.frame.last_end_frame_ms)
                .unwrap_or(0.0);
            backend_gpu_timestamps_enabled = backend_timing_snapshot
                .as_ref()
                .map(|diag| diag.frame.gpu_timestamps_enabled)
                .unwrap_or(false);
            backend_gpu_timing_frame_index = backend_timing_snapshot
                .as_ref()
                .map(|diag| diag.frame.last_gpu_timing_frame_index)
                .unwrap_or(0);
            backend_gpu_shadow_ms = backend_timing_snapshot
                .as_ref()
                .map(|diag| diag.frame.last_gpu_shadow_ms)
                .unwrap_or(0.0);
            backend_gpu_opaque_ms = backend_timing_snapshot
                .as_ref()
                .map(|diag| diag.frame.last_gpu_opaque_ms)
                .unwrap_or(0.0);
            backend_gpu_postfx_ms = backend_timing_snapshot
                .as_ref()
                .map(|diag| diag.frame.last_gpu_postfx_ms)
                .unwrap_or(0.0);
            backend_gpu_ui_ms = backend_timing_snapshot
                .as_ref()
                .map(|diag| diag.frame.last_gpu_ui_ms)
                .unwrap_or(0.0);
            backend_gpu_profiled_ms = backend_timing_snapshot
                .as_ref()
                .map(|diag| diag.frame.last_gpu_profiled_ms)
                .unwrap_or(0.0);
            self.bridge_render_backend_events(ctx, &mut **r);

            if let Some(snapshot) = frame_debug_snapshot.take() {
                self.diagnostics
                    .overlay_metrics
                    .publish_debug_snapshot(snapshot);
                let telemetry = self.diagnostics.overlay_metrics.telemetry_snapshot();
                if runtime_debug_overlay_enabled() {
                    let overlay_text = self.diagnostics.overlay_metrics.overlay_text();
                    let ui_telemetry =
                        UiRuntimeDebugOverlayTelemetry::new(self.frame.frame_index, overlay_text)
                            .with_metric(
                                "render_debug",
                                serde_json::to_value(&telemetry).unwrap_or(serde_json::Value::Null),
                            );
                    ui_telemetry_to_publish = Some(ui_telemetry);
                }
                telemetry_to_publish = Some(telemetry);
            }
            self.trace_render_diagnostics(&mut **r, trace_frame);
        }

        drop(r);
        ctx.resources_mut().insert(SceneLaunchStatus::inactive());
        if let Some(telemetry) = telemetry_to_publish {
            ctx.resources_mut().insert(telemetry);
        }
        if let Some(ui_telemetry) = ui_telemetry_to_publish {
            ctx.resources_mut().insert(ui_telemetry);
        } else {
            let _ = ctx
                .resources_mut()
                .remove::<UiRuntimeDebugOverlayTelemetry>();
        }
        let render_timing = newengine_core::render::RenderModuleTimingTelemetry {
            frame_index: self.frame.frame_index,
            total_ms: render_module_started.elapsed().as_secs_f64() * 1000.0,
            pre_begin_ms,
            backend_begin_ms,
            playable_frame_ms,
            diagnostics_before_present_ms,
            backend_end_ms,
            backend_reported_begin_ms,
            backend_frame_slot_wait_ms,
            backend_surface_acquire_ms,
            backend_image_wait_ms,
            backend_reported_end_ms,
            backend_gpu_timestamps_enabled,
            backend_gpu_timing_frame_index,
            backend_gpu_shadow_ms,
            backend_gpu_opaque_ms,
            backend_gpu_postfx_ms,
            backend_gpu_ui_ms,
            backend_gpu_profiled_ms,
        };
        if newengine_runtime_policy::render_runtime_policy().render_phase_log
            && render_timing.frame_index.is_multiple_of(60)
        {
            newengine_ulog_api::ulog::info!(
                "render phase profile: frame={} total_ms={:.3} pre_begin_ms={:.3} backend_begin_ms={:.3} playable_ms={:.3} diagnostics_ms={:.3} backend_end_ms={:.3} vk_begin_ms={:.3} slot_wait_ms={:.3} acquire_ms={:.3} image_wait_ms={:.3} vk_end_ms={:.3} gpu_ts={} gpu_frame={} gpu_shadow_ms={:.3} gpu_opaque_ms={:.3} gpu_postfx_ms={:.3} gpu_ui_ms={:.3} gpu_profiled_ms={:.3}",
                render_timing.frame_index,
                render_timing.total_ms,
                render_timing.pre_begin_ms,
                render_timing.backend_begin_ms,
                render_timing.playable_frame_ms,
                render_timing.diagnostics_before_present_ms,
                render_timing.backend_end_ms,
                render_timing.backend_reported_begin_ms,
                render_timing.backend_frame_slot_wait_ms,
                render_timing.backend_surface_acquire_ms,
                render_timing.backend_image_wait_ms,
                render_timing.backend_reported_end_ms,
                render_timing.backend_gpu_timestamps_enabled,
                render_timing.backend_gpu_timing_frame_index,
                render_timing.backend_gpu_shadow_ms,
                render_timing.backend_gpu_opaque_ms,
                render_timing.backend_gpu_postfx_ms,
                render_timing.backend_gpu_ui_ms,
                render_timing.backend_gpu_profiled_ms,
            );
        }
        trace_backend_phase_graph_dump(render_timing.frame_index, trace_frame);
        ctx.resources_mut().insert(render_timing);
        Ok(())
    }

    fn sync_plugin_bridge<E: Send + 'static>(
        &mut self,
        ctx: &mut ModuleCtx<'_, E>,
        snapshot: Option<&newengine_plugin_host::PluginsSnapshot>,
    ) {
        if let Some(snap) = snapshot {
            let _ = self.bridges.plugins.publish_if_changed(snap);
        }
        if let Some(q) = ctx
            .resources_mut()
            .get_mut::<newengine_plugin_host::PluginControlQueue>()
        {
            for cmd in self.bridges.plugins.drain_cmds() {
                q.push(cmd);
            }
        }
    }

    fn apply_editor_viewport_slot<E: Send + 'static>(
        &self,
        ctx: &ModuleCtx<'_, E>,
        surface_w: u32,
        surface_h: u32,
    ) {
        if self.bridges.viewport.external_extent_owned() {
            return;
        }
        let Some(slot) = ctx.resources().get::<UiViewportSlot>() else {
            return;
        };
        let (mut vp_w, mut vp_h) = slot.extent_px();
        if vp_w == 0 || vp_h == 0 {
            self.bridges.viewport.publish_extent(0, 0);
            return;
        }
        vp_w = vp_w.min(surface_w.max(1));
        vp_h = vp_h.min(surface_h.max(1));
        self.bridges.viewport.publish_extent(vp_w, vp_h);
    }

    fn begin_playable_surface_frame(
        &mut self,
        r: &mut dyn newengine_core::render::RenderApi,
        ui_enabled: bool,
        w: u32,
        h: u32,
        dt: f32,
        fixed_dt: f32,
        fixed_alpha: f32,
        fixed_step_count: u32,
        fixed_tick: u64,
        trace_frame: bool,
    ) -> EngineResult<Option<RenderFrameScope>> {
        let (requested_vp_w, requested_vp_h) = self.bridges.viewport.read_extent();
        let direct_surface_viewport = requested_vp_w == 0 && requested_vp_h == 0 && w > 0 && h > 0;
        let (vp_w, vp_h) = if direct_surface_viewport {
            (w, h)
        } else {
            (requested_vp_w, requested_vp_h)
        };

        let scene_clear_color = self
            .bridges
            .scene
            .scene()
            .read()
            .world()
            .resource::<WorldClearColor>()
            .map(|sky| sky.color);
        self.viewport.clear_color = resolve_viewport_clear_color(
            self.external_preview_target_active(),
            scene_clear_color,
            self.runtime_profile().configured_clear_color(),
        );
        self.trace_begin_frame(trace_frame, vp_w, vp_h);
        r.begin_frame(
            BeginFrameDesc::new(self.viewport.clear_color)
                .with_frame_index(self.frame.frame_index.saturating_add(1).max(1)),
        )?;
        self.trace_begin_frame_done(trace_frame);

        Ok(Some(RenderFrameScope {
            w,
            h,
            vp_w,
            vp_h,
            direct_surface_viewport,
            ui_enabled,
            trace_frame,
            dt,
            fixed_dt,
            fixed_alpha,
            fixed_step_count,
            fixed_tick,
        }))
    }
}

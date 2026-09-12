use std::path::PathBuf;

use newengine_core::render::{RenderApiRef, RENDER_API_ID, RENDER_API_PROVIDE};
use newengine_core::{EngineResult, Module, ModuleCtx};

use crate::client::RenderServiceClient;
use crate::service_api::ServiceBackedRenderApi;
use crate::types::{ResolvedRenderBackendConfig, RENDER_BACKEND_SERVICE_SPEC};
use newengine_runtime_adapter_core::bind_backend_info;

pub struct RenderBackendRuntimeModule {
    _modules_dir: PathBuf,
    api: Option<RenderApiRef>,
}

impl RenderBackendRuntimeModule {
    #[inline]
    pub fn new(modules_dir: PathBuf) -> Self {
        Self {
            _modules_dir: modules_dir,
            api: None,
        }
    }
}

impl<E: Send + 'static> Module<E> for RenderBackendRuntimeModule {
    fn id(&self) -> &'static str {
        "render.runtime.loader"
    }

    fn provides(&self) -> &'static [newengine_core::ApiProvide] {
        &[RENDER_API_PROVIDE]
    }

    fn init(&mut self, ctx: &mut ModuleCtx<'_, E>) -> EngineResult<()> {
        let host = newengine_plugin_host::default_host_api();
        let client = RenderServiceClient::new(host);

        let (info, selection) = match bind_backend_info(
            ctx,
            RENDER_BACKEND_SERVICE_SPEC,
            client.info(),
        ) {
            Ok(bound) => bound,
            Err(err) => {
                newengine_ulog_api::ulog::warn!(
                    "render backend: unavailable; render API not registered and runtime will degrade without rendering: {}",
                    err
                );
                return Ok(());
            }
        };
        let protocol_version = info.protocol_version;
        let expected_protocol = newengine_core::render::RenderApiVersion::default();
        if !expected_protocol.is_major_compatible_with(protocol_version) {
            newengine_ulog_api::ulog::error!(
                "render backend: refusing incompatible provider protocol=v{}.{}.{} engine=v{}.{}.{} provider='{}'",
                protocol_version.major,
                protocol_version.minor,
                protocol_version.patch,
                expected_protocol.major,
                expected_protocol.minor,
                expected_protocol.patch,
                selection.provider_plugin_id,
            );
            return Ok(());
        }
        let negotiation = match client
            .negotiate(newengine_core::render::RenderCapabilityNegotiationRequest::default())
        {
            Ok(response) => response,
            Err(error) => {
                newengine_ulog_api::ulog::error!(
                    "render backend: protocol negotiation failed provider='{}': {}",
                    selection.provider_plugin_id,
                    error
                );
                return Ok(());
            }
        };
        if !negotiation.ok
            || !expected_protocol.is_major_compatible_with(negotiation.accepted_version)
            || !expected_protocol.is_major_compatible_with(negotiation.backend_version)
        {
            let notices = negotiation
                .notices
                .iter()
                .map(|notice| format!("{}: {}", notice.code, notice.message))
                .collect::<Vec<_>>()
                .join("; ");
            newengine_ulog_api::ulog::error!(
                "render backend: negotiation rejected provider='{}' accepted=v{}.{}.{} backend=v{}.{}.{} notices='{}'",
                selection.provider_plugin_id,
                negotiation.accepted_version.major,
                negotiation.accepted_version.minor,
                negotiation.accepted_version.patch,
                negotiation.backend_version.major,
                negotiation.backend_version.minor,
                negotiation.backend_version.patch,
                notices
            );
            return Ok(());
        }

        let negotiated_capabilities = negotiation.capabilities.clone();
        newengine_ulog_api::ulog::info!(
            "render backend: service bridge bound id='{}' name='{}' version='{}' provider='{}' provider_state='{}' matched_by='{}' debug_text='{}' protocol=v{}.{}.{} features={} hardware_tier={:?} frames_in_flight={} completion_events={} device_loss={:?} device_recreate={} resource_replay={} upload_budget={}MB/frame",
            info.backend_id,
            info.backend_name,
            info.backend_version,
            selection.provider_plugin_id,
            selection.provider_state,
            selection.matched_by,
            info.debug_text,
            protocol_version.major,
            protocol_version.minor,
            protocol_version.patch,
            negotiated_capabilities.features.len(),
            negotiated_capabilities.hardware_tier,
            negotiated_capabilities.execution.normalized_frames_in_flight(),
            negotiated_capabilities.execution.frame_completion_events,
            negotiated_capabilities.execution.device_loss,
            negotiated_capabilities.execution.recovery.device_recreate,
            negotiated_capabilities.execution.recovery.resource_replay,
            info.work_budget.max_upload_bytes_per_frame / (1024 * 1024)
        );

        if std::env::var_os("NEWENGINE_RENDER_CAPS_DUMP").is_some() {
            newengine_ulog_api::ulog::info!(
                "render backend: negotiated feature surface {:?}",
                negotiated_capabilities.features
            );
        }

        let resolved = ResolvedRenderBackendConfig {
            backend_id: info.backend_id,
            clear_color: info.clear_color,
            debug_text: info.debug_text,
            capabilities: negotiated_capabilities,
            work_budget: info.work_budget,
        };

        let api = RenderApiRef::new(ServiceBackedRenderApi::new(client));

        ctx.resources_mut().insert(resolved);
        ctx.resources_mut()
            .register_api(RENDER_API_ID, api.clone())?;
        self.api = Some(api);

        Ok(())
    }

    fn shutdown(&mut self, ctx: &mut ModuleCtx<'_, E>) -> EngineResult<()> {
        let _ = ctx
            .resources_mut()
            .unregister_api::<RenderApiRef>(RENDER_API_ID);
        let _ = ctx.resources_mut().remove::<ResolvedRenderBackendConfig>();
        self.api = None;
        Ok(())
    }
}

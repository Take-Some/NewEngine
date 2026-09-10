#![forbid(unsafe_op_in_unsafe_fn)]

mod adapter;
mod cap_validate;
mod config_diff;
mod config_patch;
mod discovery;
mod lifecycle;
mod load_profile;
mod loader;
mod plugin_init;
mod types;
mod ui_assets;

pub(crate) use discovery::read_verified_manifest;
pub use discovery::{
    resolve_plugin_discovery_dir, scan_plugin_discovery_graph,
    DiscoveryGraph as PluginDiscoveryGraph, IncrementalLoadOutcome,
    PluginRuntimeUnitInventoryEntry,
};
pub use types::{PluginIconSnapshot, PluginLoadError, PluginLoadOrigin, PluginSnapshotEntry};

use newengine_math::collections::prelude::*;

use self::types::LoadedPlugin;

pub struct PluginManager {
    host: crate::host_context::HostContextHandle,
    loaded: Vec<LoadedPlugin>,
    loaded_ids: NeHashSet<String>,
    discovery_cache: Option<discovery::DiscoveryGraph>,
    composition_discovery_cache: NeHashMap<std::path::PathBuf, discovery::DiscoveryGraph>,
    targeted_discovery_cache: Option<discovery::TargetedDiscoveryInventory>,
    frozen_composition_plan: Option<discovery::FrozenPluginCompositionPlan>,
    incremental_load: Option<discovery::IncrementalLoadState>,
}

impl PluginManager {
    #[inline]
    pub fn new() -> Self {
        Self::new_with_host(crate::host_context::create_host_context())
    }

    #[inline]
    pub fn new_with_host(host: crate::host_context::HostContextHandle) -> Self {
        Self {
            host,
            loaded: Vec::new(),
            loaded_ids: NeHashSet::default(),
            discovery_cache: None,
            composition_discovery_cache: NeHashMap::default(),
            targeted_discovery_cache: None,
            frozen_composition_plan: None,
            incremental_load: None,
        }
    }

    #[inline]
    pub fn host_context(&self) -> &crate::host_context::HostContextHandle {
        &self.host
    }

    #[inline]
    pub fn has_plugin(&self, id: &str) -> bool {
        self.loaded_ids.contains(id)
    }

    #[inline]
    pub fn find_index(&self, id: &str) -> Option<usize> {
        self.loaded.iter().position(|p| p.info.id.as_str() == id)
    }

    #[inline]
    pub fn snapshot(&self) -> Vec<PluginSnapshotEntry> {
        crate::host_context::with_host_context(&self.host, || {
            let mut out = types::snapshot_impl(&self.loaded);
            out.extend(
                crate::host_context::list_external_runtime_plugins()
                    .into_iter()
                    .map(|p| PluginSnapshotEntry {
                        path: p.path,
                        id: p.id,
                        name: p.name,
                        version: p.version,
                        kind: p.kind,
                        capabilities: p.capabilities,
                        state: p.state,
                        disabled_reason: p.disabled_reason,
                        icon_small: None,
                    }),
            );
            out.sort_by(|a, b| a.id.cmp(&b.id));
            out
        })
    }
}

impl Default for PluginManager {
    #[inline]
    fn default() -> Self {
        Self::new()
    }
}

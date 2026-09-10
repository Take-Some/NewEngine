#![forbid(unsafe_op_in_unsafe_fn)]

use std::path::{Path, PathBuf};

use newengine_plugin_api::HostApiV1;

use super::graph::{DiscoveryGraph, LoadPhaseFilter};
use super::logging::{emit_discovery_logs, emit_selection_table};
use super::scan::{scan_plugin_id_from_inventory, scan_plugins_dir, scan_targeted_inventory};
use super::selection::{build_frozen_composition_plan, build_load_selection};
use crate::manager::types::{PluginLoadError, PluginLoadOrigin};
use crate::paths::{default_plugins_dir, resolve_plugins_dir};
use crate::PluginManager;
use newengine_ulog_api::formatting::emit_boxed_kv;
use newengine_ulog_api::path_format::{canonicalize_if_exists, display_clean};

pub fn resolve_plugin_discovery_dir(dir: Option<&Path>) -> Result<PathBuf, PluginLoadError> {
    let dir = match dir {
        Some(dir) => resolve_plugins_dir(dir)?,
        None => default_plugins_dir()?,
    };

    if let Err(e) = std::fs::create_dir_all(&dir) {
        return Err(PluginLoadError {
            path: dir.clone(),
            message: format!("create_dir_all failed: {e}"),
        });
    }

    Ok(canonicalize_if_exists(&dir))
}

pub fn scan_plugin_discovery_graph(dir: &Path) -> Result<DiscoveryGraph, PluginLoadError> {
    let dir = resolve_plugin_discovery_dir(Some(dir))?;
    scan_plugins_dir(&dir)
}

#[derive(Debug, Clone)]
pub struct IncrementalLoadOutcome {
    pub finished: bool,
    pub loaded_total: usize,
    pub loaded_this_phase: usize,
    pub pending_total: usize,
    pub completed: usize,
    pub load_errors: usize,
    pub progress_01: f32,
    pub current_path: Option<PathBuf>,
}

impl IncrementalLoadOutcome {
    #[inline]
    fn running(
        loaded_total: usize,
        loaded_this_phase: usize,
        pending_total: usize,
        completed: usize,
        load_errors: usize,
        current_path: Option<PathBuf>,
    ) -> Self {
        let progress_01 = if pending_total == 0 {
            1.0
        } else {
            (completed as f32 / pending_total as f32).clamp(0.0, 1.0)
        };
        Self {
            finished: false,
            loaded_total,
            loaded_this_phase,
            pending_total,
            completed,
            load_errors,
            progress_01,
            current_path,
        }
    }

    #[inline]
    fn finished(
        loaded_total: usize,
        loaded_this_phase: usize,
        pending_total: usize,
        load_errors: usize,
    ) -> Self {
        Self {
            finished: true,
            loaded_total,
            loaded_this_phase,
            pending_total,
            completed: pending_total,
            load_errors,
            progress_01: 1.0,
            current_path: None,
        }
    }
}

pub(crate) struct IncrementalLoadState {
    graph: DiscoveryGraph,
    graph_is_new: bool,
    selection: super::selection::LoadSelection,
    filter: LoadPhaseFilter,
    pending: Vec<PathBuf>,
    next_index: usize,
    loaded_ids_before_len: usize,
    load_errors: Vec<PluginLoadError>,
    strict: bool,
    load_origin: PluginLoadOrigin,
}

mod bulk;
mod entrypoints;
mod incremental;
mod targeted;

#![forbid(unsafe_op_in_unsafe_fn)]
mod graph;
mod load;
mod logging;
mod metadata;
mod scan;
mod selection;
mod sidecar;

pub use self::graph::{DiscoveryGraph, PluginRuntimeUnitInventoryEntry};
pub(super) use self::load::IncrementalLoadState;
pub use self::load::{
    resolve_plugin_discovery_dir, scan_plugin_discovery_graph, IncrementalLoadOutcome,
};
pub(super) use self::scan::TargetedDiscoveryInventory;
pub(super) use self::selection::FrozenPluginCompositionPlan;

pub(crate) use self::sidecar::read_verified_manifest;
pub(super) use self::sidecar::{
    verify_artifact_against_manifest, verify_live_descriptor_against_manifest,
};

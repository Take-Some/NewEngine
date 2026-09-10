#![forbid(unsafe_op_in_unsafe_fn)]

use newengine_math::collections_prelude::NeHashMap as HashMap;
use std::path::{Path, PathBuf};

use super::graph::{DiscoveryGraph, ScannedDynlib, ScannedDynlibKind};
use super::metadata::{build_scanned_plugin_kind, ScanPluginProbe};
use super::sidecar::{read_manifest_metadata, read_verified_manifest};
use crate::manager::types::PluginLoadError;
use crate::paths::is_dynamic_lib;
use newengine_ulog_api::path_format::display_clean;

pub(super) fn scan_plugins_dir(dir: &Path) -> Result<DiscoveryGraph, PluginLoadError> {
    let rd = std::fs::read_dir(dir).map_err(|e| PluginLoadError {
        path: dir.to_path_buf(),
        message: format!("read_dir failed: {e}"),
    })?;

    let mut entries_total: usize = 0;
    let mut skipped_non_dynlib: usize = 0;
    let mut dynlib_paths: Vec<PathBuf> = Vec::new();
    let mut scan_errors: Vec<String> = Vec::new();
    for ent in rd {
        entries_total = entries_total.saturating_add(1);

        let ent = ent.map_err(|e| PluginLoadError {
            path: dir.to_path_buf(),
            message: format!("read_dir entry failed: {e}"),
        })?;

        let path = ent.path();
        if !is_dynamic_lib(&path) {
            skipped_non_dynlib = skipped_non_dynlib.saturating_add(1);
            continue;
        }

        dynlib_paths.push(path);
    }

    dynlib_paths.sort_by_key(|a| sort_key(a));

    let mut items: Vec<ScannedDynlib> = Vec::with_capacity(dynlib_paths.len());

    for path in dynlib_paths {
        match scan_dynamic_lib(&path) {
            Ok(v) => items.push(v),
            Err(e) => {
                newengine_ulog_api::ulog::warn!(
                    "plugins: scan failed for '{}': {}",
                    display_clean(&path),
                    e
                );
                scan_errors.push(format!("{}: {}", display_clean(&path), e));
            }
        }
    }

    items.sort_by_key(|item| sort_key(&item.path));

    let mut platform_runtime_count = 0usize;
    let mut bootstrap_total = 0usize;
    let mut engine_total = 0usize;
    let mut unknown_dynlibs: Vec<String> = Vec::new();

    for item in &items {
        match &item.kind {
            ScannedDynlibKind::PlatformRuntime { .. } => {
                platform_runtime_count = platform_runtime_count.saturating_add(1);
            }
            ScannedDynlibKind::Plugin { phase, .. } => match phase {
                newengine_plugin_api::PluginBootstrapPhase::Bootstrap => {
                    bootstrap_total = bootstrap_total.saturating_add(1);
                }
                newengine_plugin_api::PluginBootstrapPhase::Platform
                | newengine_plugin_api::PluginBootstrapPhase::Engine => {
                    engine_total = engine_total.saturating_add(1);
                }
            },
            ScannedDynlibKind::Unknown => {
                unknown_dynlibs.push(item.file_name.clone());
            }
        }
    }

    Ok(DiscoveryGraph {
        dir: dir.to_path_buf(),
        entries_total,
        skipped_non_dynlib,
        items,
        scan_errors,
        platform_runtime_count,
        bootstrap_total,
        engine_total,
        unknown_dynlibs,
    })
}

#[derive(Debug, Clone)]
pub(crate) struct TargetedDiscoveryInventory {
    pub(super) dir: PathBuf,
    pub(super) entries_total: usize,
    skipped_non_dynlib: usize,
    candidate_paths: HashMap<String, Vec<PathBuf>>,
    scan_errors: Vec<String>,
}

impl TargetedDiscoveryInventory {
    #[inline]
    pub(super) fn candidate_id_count(&self) -> usize {
        self.candidate_paths.len()
    }
}

/// Reads lightweight identity metadata for every runtime DLL once, allowing a
/// sequence of targeted plugin-id lookups to reuse the same directory inventory.
/// Artifact SHA-256 remains deferred until a candidate is actually selected.
pub(super) fn scan_targeted_inventory(
    dir: &Path,
) -> Result<TargetedDiscoveryInventory, PluginLoadError> {
    let rd = std::fs::read_dir(dir).map_err(|e| PluginLoadError {
        path: dir.to_path_buf(),
        message: format!("read_dir failed: {e}"),
    })?;
    let mut entries_total = 0usize;
    let mut skipped_non_dynlib = 0usize;
    let mut candidate_paths: HashMap<String, Vec<PathBuf>> = HashMap::default();
    let mut scan_errors = Vec::<String>::new();

    for ent in rd {
        entries_total = entries_total.saturating_add(1);
        let ent = ent.map_err(|e| PluginLoadError {
            path: dir.to_path_buf(),
            message: format!("read_dir entry failed: {e}"),
        })?;
        let path = ent.path();
        if !is_dynamic_lib(&path) {
            skipped_non_dynlib = skipped_non_dynlib.saturating_add(1);
            continue;
        }
        let manifest = match read_manifest_metadata(&path) {
            Ok(manifest) => manifest,
            Err(error) => {
                scan_errors.push(format!("{}: {}", display_clean(&path), error));
                continue;
            }
        };
        let signature_id = manifest
            .signature
            .as_ref()
            .map(|signature| signature.id.as_str());
        if let Some(signature) = manifest.signature.as_ref() {
            candidate_paths
                .entry(signature.id.clone())
                .or_default()
                .push(path.clone());
        }
        if let Some(descriptor) = manifest.descriptor.as_ref() {
            if signature_id != Some(descriptor.id.as_str()) {
                candidate_paths
                    .entry(descriptor.id.clone())
                    .or_default()
                    .push(path.clone());
            }
        }
    }
    for paths in candidate_paths.values_mut() {
        paths.sort_by_key(|path| sort_key(path));
    }
    Ok(TargetedDiscoveryInventory {
        dir: dir.to_path_buf(),
        entries_total,
        skipped_non_dynlib,
        candidate_paths,
        scan_errors,
    })
}

/// Discovers candidates for one explicit plugin id without hashing unrelated DLLs.
///
/// The sidecar is inventory metadata, so reading it is enough to eliminate siblings
/// that cannot satisfy the requested identity. Matching artifacts are then routed
/// through `scan_dynamic_lib`, preserving the normal SHA-256 and descriptor checks.
#[cfg(test)]
pub(super) fn scan_plugin_id(
    dir: &Path,
    plugin_id: &str,
) -> Result<DiscoveryGraph, PluginLoadError> {
    let inventory = scan_targeted_inventory(dir)?;
    scan_plugin_id_from_inventory(&inventory, plugin_id)
}

pub(super) fn scan_plugin_id_from_inventory(
    inventory: &TargetedDiscoveryInventory,
    plugin_id: &str,
) -> Result<DiscoveryGraph, PluginLoadError> {
    let plugin_id = plugin_id.trim();
    let matching_paths = inventory
        .candidate_paths
        .get(plugin_id)
        .cloned()
        .unwrap_or_default();
    let had_matching_paths = !matching_paths.is_empty();
    let mut matching_failures = Vec::<String>::new();
    let mut scan_errors = inventory.scan_errors.clone();
    let mut items = Vec::<ScannedDynlib>::with_capacity(matching_paths.len());
    for path in matching_paths {
        match scan_dynamic_lib(&path) {
            Ok(item) => items.push(item),
            Err(error) => {
                let detail = format!("{}: {}", display_clean(&path), error);
                matching_failures.push(detail.clone());
                scan_errors.push(detail);
            }
        }
    }

    if had_matching_paths && items.is_empty() {
        return Err(PluginLoadError {
            path: inventory.dir.clone(),
            message: format!(
                "targeted discovery found plugin id '{}' but every matching artifact failed verification: {}",
                plugin_id,
                matching_failures.join(" | ")
            ),
        });
    }

    let bootstrap_total = items
        .iter()
        .filter(|item| {
            matches!(
                &item.kind,
                ScannedDynlibKind::Plugin {
                    phase: newengine_plugin_api::PluginBootstrapPhase::Bootstrap,
                    ..
                }
            )
        })
        .count();
    let engine_total = items.len().saturating_sub(bootstrap_total);

    Ok(DiscoveryGraph {
        dir: inventory.dir.clone(),
        entries_total: inventory.entries_total,
        skipped_non_dynlib: inventory.skipped_non_dynlib,
        items,
        scan_errors,
        platform_runtime_count: 0,
        bootstrap_total,
        engine_total,
        unknown_dynlibs: Vec::new(),
    })
}

fn scan_dynamic_lib(path: &Path) -> Result<ScannedDynlib, String> {
    let file_name = file_name_only(path);
    let manifest = read_verified_manifest(path)?;
    let metadata = &manifest.manifest;

    if let Some(platform) = metadata.platform_runtime.as_ref() {
        return Ok(ScannedDynlib {
            path: path.to_path_buf(),
            file_name,
            discovery_manifest: Some(manifest.clone()),
            kind: ScannedDynlibKind::PlatformRuntime {
                id: platform.id.clone(),
                version: platform.version.clone(),
                system_tags: platform.system_tags.clone(),
                backend_priority: platform.backend_priority,
            },
        });
    }

    let signature = metadata
        .signature
        .as_ref()
        .map(|signature| {
            Ok::<newengine_plugin_api::PluginSignatureV1, String>(
                newengine_plugin_api::PluginSignatureV1 {
                    id: signature.id.clone().into(),
                    name: signature.name.clone().into(),
                    version: signature.version.clone().into(),
                    kind: newengine_plugin_api::plugin_kind_from_u8(signature.kind)?,
                    bootstrap_phase: newengine_plugin_api::bootstrap_phase_from_u8(
                        signature.bootstrap_phase,
                    )?,
                },
            )
        })
        .transpose()?;
    let descriptor_v2 = metadata
        .descriptor
        .as_ref()
        .map(newengine_plugin_api::PluginDiscoveryDescriptorV1::to_descriptor_v2)
        .transpose()?;
    let probe = ScanPluginProbe {
        signature,
        info: None,
        descriptor: None,
        descriptor_v2,
        has_canonical_root: metadata.has_canonical_root,
        has_legacy_root: metadata.has_legacy_root,
    };

    if let Some(kind) = build_scanned_plugin_kind(&probe) {
        return Ok(ScannedDynlib {
            path: path.to_path_buf(),
            file_name,
            discovery_manifest: Some(manifest.clone()),
            kind,
        });
    }

    Ok(ScannedDynlib {
        path: path.to_path_buf(),
        file_name,
        discovery_manifest: Some(manifest),
        kind: ScannedDynlibKind::Unknown,
    })
}

#[inline]
fn file_name_only(path: &Path) -> String {
    path.file_name()
        .and_then(|s| s.to_str())
        .map(str::to_owned)
        .unwrap_or_else(|| "<unnamed>".to_owned())
}

#[inline]
fn sort_key(path: &Path) -> (String, String) {
    let file_name = path
        .file_name()
        .and_then(|s| s.to_str())
        .map(str::to_owned)
        .unwrap_or_else(|| "<unnamed>".to_owned());
    (file_name, display_clean(path))
}

#[cfg(test)]
mod tests {
    use super::*;
    use newengine_plugin_api::{
        bootstrap_phase_to_u8, plugin_kind_to_u8, PluginBootstrapPhase, PluginDescriptorV2,
        PluginDiscoveryDescriptorV1, PluginDiscoveryManifestV2, PluginDiscoverySignatureV1,
        PluginKind, PLUGIN_DISCOVERY_MANIFEST_SCHEMA_VERSION,
    };
    use sha2::{Digest, Sha256};
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn targeted_discovery_does_not_hash_unrelated_artifacts() {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock")
            .as_nanos();
        let dir = std::env::temp_dir().join(format!("northstar-targeted-discovery-{nonce}"));
        std::fs::create_dir_all(&dir).expect("temp dir");

        let write_fixture = |file_name: &str, id: &str, bytes: &[u8]| {
            let dll = dir.join(file_name);
            std::fs::write(&dll, bytes).expect("fake dll");
            let descriptor = PluginDescriptorV2 {
                id: id.into(),
                name: id.into(),
                version: "1.0.0".into(),
                kind: PluginKind::Runtime,
                capabilities: Vec::new().into(),
                extension_json: "".into(),
            };
            let manifest = PluginDiscoveryManifestV2 {
                schema_version: PLUGIN_DISCOVERY_MANIFEST_SCHEMA_VERSION,
                artifact_file: file_name.to_owned(),
                signature: Some(PluginDiscoverySignatureV1 {
                    id: id.to_owned(),
                    name: id.to_owned(),
                    version: "1.0.0".to_owned(),
                    kind: plugin_kind_to_u8(PluginKind::Runtime),
                    bootstrap_phase: bootstrap_phase_to_u8(PluginBootstrapPhase::Engine),
                }),
                descriptor: Some(PluginDiscoveryDescriptorV1::from_descriptor_v2(&descriptor)),
                platform_runtime: None,
                has_canonical_root: true,
                has_legacy_root: false,
            };
            let sidecar =
                dll.with_extension(newengine_plugin_api::PLUGIN_DISCOVERY_MANIFEST_SUFFIX);
            std::fs::write(sidecar, serde_json::to_vec_pretty(&manifest).expect("json"))
                .expect("sidecar");
        };

        write_fixture("target.dll", "test.target.provider", b"target-artifact");
        // Targeted lookup reads only the unrelated provider sidecar; its payload is not
        // fingerprinted because the provider id does not match the request.
        write_fixture(
            "unrelated.dll",
            "test.unrelated.provider",
            b"unrelated-artifact",
        );

        let graph =
            scan_plugin_id(&dir, "test.target.provider").expect("targeted discovery succeeds");
        assert_eq!(graph.items.len(), 1);
        match &graph.items[0].kind {
            ScannedDynlibKind::Plugin { id, .. } => assert_eq!(id, "test.target.provider"),
            other => panic!("expected plugin, got {other:?}"),
        }

        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn scan_captures_artifact_fingerprint_in_memory() {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock")
            .as_nanos();
        let dir = std::env::temp_dir().join(format!("northstar-scan-fingerprint-{nonce}"));
        std::fs::create_dir_all(&dir).expect("temp dir");
        let dll = dir.join("target.dll");
        let bytes = b"target-artifact";
        std::fs::write(&dll, bytes).expect("fake dll");
        let descriptor = PluginDescriptorV2 {
            id: "test.target.provider".into(),
            name: "Target".into(),
            version: "1.0.0".into(),
            kind: PluginKind::Runtime,
            capabilities: Vec::new().into(),
            extension_json: "".into(),
        };
        let manifest = PluginDiscoveryManifestV2 {
            schema_version: PLUGIN_DISCOVERY_MANIFEST_SCHEMA_VERSION,
            artifact_file: "target.dll".to_owned(),
            signature: Some(PluginDiscoverySignatureV1 {
                id: "test.target.provider".to_owned(),
                name: "Target".to_owned(),
                version: "1.0.0".to_owned(),
                kind: plugin_kind_to_u8(PluginKind::Runtime),
                bootstrap_phase: bootstrap_phase_to_u8(PluginBootstrapPhase::Engine),
            }),
            descriptor: Some(PluginDiscoveryDescriptorV1::from_descriptor_v2(&descriptor)),
            platform_runtime: None,
            has_canonical_root: true,
            has_legacy_root: false,
        };
        std::fs::write(
            dll.with_extension(newengine_plugin_api::PLUGIN_DISCOVERY_MANIFEST_SUFFIX),
            serde_json::to_vec_pretty(&manifest).expect("json"),
        )
        .expect("sidecar");

        let scanned = scan_dynamic_lib(&dll).expect("scan");
        let snapshot = scanned.discovery_manifest.expect("verified snapshot");
        assert_eq!(snapshot.manifest, manifest);
        assert_eq!(snapshot.artifact_size, bytes.len() as u64);
        assert_eq!(
            snapshot.artifact_sha256,
            format!("{:x}", Sha256::digest(bytes))
        );

        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn discovery_uses_sidecar_without_mapping_binary() {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock")
            .as_nanos();
        let dir = std::env::temp_dir().join(format!("northstar-zero-map-{nonce}"));
        std::fs::create_dir_all(&dir).expect("temp dir");
        let dll = dir.join("fake-provider.dll");
        // Deliberately not a PE/ELF/Mach-O image. Any Library::new() in discovery
        // would make this regression fail.
        let bytes = b"northstar-sidecar-only-not-a-dynamic-library";
        std::fs::write(&dll, bytes).expect("fake dll");

        let descriptor = PluginDescriptorV2 {
            id: "test.fake.provider".into(),
            name: "Fake Provider".into(),
            version: "1.2.3".into(),
            kind: PluginKind::Runtime,
            capabilities: Vec::new().into(),
            extension_json: "".into(),
        };
        let manifest = PluginDiscoveryManifestV2 {
            schema_version: PLUGIN_DISCOVERY_MANIFEST_SCHEMA_VERSION,
            artifact_file: "fake-provider.dll".to_owned(),
            signature: Some(PluginDiscoverySignatureV1 {
                id: "test.fake.provider".to_owned(),
                name: "Fake Provider".to_owned(),
                version: "1.2.3".to_owned(),
                kind: plugin_kind_to_u8(PluginKind::Runtime),
                bootstrap_phase: bootstrap_phase_to_u8(PluginBootstrapPhase::Engine),
            }),
            descriptor: Some(PluginDiscoveryDescriptorV1::from_descriptor_v2(&descriptor)),
            platform_runtime: None,
            has_canonical_root: true,
            has_legacy_root: false,
        };
        let sidecar = dll.with_extension(newengine_plugin_api::PLUGIN_DISCOVERY_MANIFEST_SUFFIX);
        std::fs::write(
            &sidecar,
            serde_json::to_vec_pretty(&manifest).expect("json"),
        )
        .expect("sidecar");

        let scanned = scan_dynamic_lib(&dll).expect("sidecar-only discovery must succeed");
        assert_eq!(
            scanned
                .discovery_manifest
                .as_ref()
                .map(|snapshot| &snapshot.manifest),
            Some(&manifest)
        );
        match scanned.kind {
            ScannedDynlibKind::Plugin { id, version, .. } => {
                assert_eq!(id, "test.fake.provider");
                assert_eq!(version, "1.2.3");
            }
            other => panic!("expected plugin, got {other:?}"),
        }

        let _ = std::fs::remove_dir_all(dir);
    }
}

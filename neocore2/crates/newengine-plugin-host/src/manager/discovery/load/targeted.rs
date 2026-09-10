use super::*;

impl PluginManager {
    #[inline]
    pub fn load_path(&mut self, path: &Path, host: HostApiV1) -> Result<(), PluginLoadError> {
        self.load_one(path, host)
    }

    /// Loads one explicitly selected dynamic plugin with a host-owned trust/origin.
    /// This lets project manifests select a game DLL before runtime-profile launch
    /// without scanning unrelated sibling libraries.
    #[inline]
    pub fn load_path_with_origin(
        &mut self,
        path: &Path,
        host: HostApiV1,
        load_origin: PluginLoadOrigin,
    ) -> Result<(), PluginLoadError> {
        self.load_one_with_origin(path, host, load_origin)
    }

    /// Loads exactly one descriptor-selected plugin id from the default runtime
    /// plugin directory without initializing unrelated providers.
    ///
    /// A default-runtime targeted load is authoritative: callers use this path only
    /// after selecting a concrete plugin id (runtime composition, project browser,
    /// etc.). Returning `Ok(false)` here used to hide discovery failures and later
    /// surface as an empty composition graph. Fail closed with the selected id and
    /// resolved directory instead; the lower-level directory API retains `bool` for
    /// callers that intentionally probe optional plugin presence.
    pub fn load_plugin_id_default_with_origin(
        &mut self,
        plugin_id: &str,
        host: HostApiV1,
        load_origin: PluginLoadOrigin,
    ) -> Result<bool, PluginLoadError> {
        let dir = default_plugins_dir()?;
        let loaded =
            self.load_plugin_id_from_dir_with_origin(&dir, plugin_id, host, load_origin)?;
        if loaded {
            return Ok(true);
        }

        Err(PluginLoadError {
            path: dir,
            message: format!(
                "targeted discovery did not find selected plugin id '{}'; verify the DLL contains finalized embedded discovery metadata (or a valid .nspmeta.json fallback for dev/debug)",
                plugin_id.trim()
            ),
        })
    }

    /// Loads exactly one descriptor-selected plugin id from a discovery directory.
    /// Unlike `load_from_dir*`, this does not initialize unrelated renderer/physics/UI
    /// providers just because they share the same pluginsRuntime directory.
    pub fn load_plugin_id_from_dir_with_origin(
        &mut self,
        dir: &Path,
        plugin_id: &str,
        host: HostApiV1,
        load_origin: PluginLoadOrigin,
    ) -> Result<bool, PluginLoadError> {
        let plugin_id = plugin_id.trim();
        if plugin_id.is_empty() {
            return Ok(false);
        }
        if self.loaded_ids.contains(plugin_id) {
            return Ok(true);
        }

        let dir = resolve_plugins_dir(dir)?;
        if let Err(e) = std::fs::create_dir_all(&dir) {
            return Err(PluginLoadError {
                path: dir.clone(),
                message: format!("create_dir_all failed: {e}"),
            });
        }
        let dir = canonicalize_if_exists(&dir);
        let needs_inventory = self
            .targeted_discovery_cache
            .as_ref()
            .is_none_or(|inventory| inventory.dir != dir);
        if needs_inventory {
            self.targeted_discovery_cache = Some(scan_targeted_inventory(&dir)?);
        }
        let inventory = self
            .targeted_discovery_cache
            .as_ref()
            .expect("targeted discovery inventory must exist after initialization");
        newengine_ulog_api::ulog::debug!(
            "plugins: targeted discovery inventory hit dir='{}' plugin='{}' entries={} candidate_ids={}",
            display_clean(&inventory.dir),
            plugin_id,
            inventory.entries_total,
            inventory.candidate_id_count(),
        );
        let graph = scan_plugin_id_from_inventory(inventory, plugin_id)?;
        let selection = build_load_selection(
            &graph,
            LoadPhaseFilter::All,
            &self.loaded_ids,
            self.frozen_composition_plan.as_ref(),
        );
        let selected_paths = selection
            .bootstrap_candidates
            .iter()
            .chain(selection.engine_candidates.iter());

        let path = graph.items.iter().find_map(|item| {
            let matches_id = matches!(
                &item.kind,
                crate::manager::discovery::graph::ScannedDynlibKind::Plugin { id, .. } if id == plugin_id
            );
            if matches_id
                && selected_paths
                    .clone()
                    .any(|selected| selected == &item.path)
            {
                Some(item.path.clone())
            } else {
                None
            }
        });

        let Some(path) = path else {
            return Ok(false);
        };
        self.load_one_with_origin(&path, host, load_origin)?;
        Ok(self.loaded_ids.contains(plugin_id))
    }

    pub(super) fn ensure_discovery_graph(
        &mut self,
        dir: &Path,
    ) -> Result<(DiscoveryGraph, bool), PluginLoadError> {
        let dir = resolve_plugins_dir(dir)?;

        if let Err(e) = std::fs::create_dir_all(&dir) {
            return Err(PluginLoadError {
                path: dir.clone(),
                message: format!("create_dir_all failed: {e}"),
            });
        }

        let dir = canonicalize_if_exists(&dir);

        if let Some(graph) = &self.discovery_cache {
            if graph.dir == dir {
                newengine_ulog_api::ulog::debug!(
                    "plugins: discovery cache hit dir='{}' entries={} dynlibs={}",
                    display_clean(&graph.dir),
                    graph.entries_total,
                    graph.items.len(),
                );
                return Ok((graph.clone(), false));
            }
        }

        if let Some(graph) = self.composition_discovery_cache.get(&dir) {
            newengine_ulog_api::ulog::debug!(
                "plugins: composition discovery reuse hit dir='{}' entries={} dynlibs={}",
                display_clean(&graph.dir),
                graph.entries_total,
                graph.items.len(),
            );
            let graph = graph.clone();
            // Promote the currently requested root into the hot single-root cache so
            // subsequent phase loads avoid even the composition-map lookup/clone.
            self.discovery_cache = Some(graph.clone());
            return Ok((graph, false));
        }

        let graph = scan_plugins_dir(&dir)?;
        self.discovery_cache = Some(graph.clone());
        Ok((graph, true))
    }
}

use super::super::profile::AuthoredWorldProfile;
use serde_json::Value;

pub(super) fn log_ymap_value_summary(logical_path: &str, value: &Value) {
    let section = ymap_payload_section_label(value);
    let definitions = count_named_arrays(value, &["definitions"]);
    let placements = count_named_arrays(value, &["placements"]);
    let definition_refs = count_named_arrays(value, &["definition_refs"]);
    let prefabs = count_named_arrays(value, &["prefabs"]);
    let surface_layers = count_named_arrays(value, &["layers", "surface_layers"]);
    let surface_nodes = count_named_objects(value, &["surface"]);
    let heightmap_nodes = count_named_objects(value, &["heightmap"]);
    let streaming_nodes = count_named_objects(value, &["streaming"]);
    let profile_present = value.pointer("/map/profile").is_some() || value.get("profile").is_some();
    newengine_ulog_api::ulog::info!(
        "authored-world ymap read: semantic XML projection path='{}' section='{}' definitions={} placements={} definition_refs={} prefabs={} terrain_surface_layer_nodes={} terrain_surface_nodes={} terrain_heightmap_nodes={} terrain_streaming_nodes={} profile_present={} policy='xml metadata projected into AuthoredWorldProfile; runtime semantics stay outside generic AssetManager'",
        logical_path,
        section,
        definitions,
        placements,
        definition_refs,
        prefabs,
        surface_layers,
        surface_nodes,
        heightmap_nodes,
        streaming_nodes,
        profile_present,
    );
}

pub(super) fn log_loaded_profile_summary(
    logical_path: &str,
    source_label: &str,
    profile: &AuthoredWorldProfile,
) {
    let surface_layers = profile.terrain.surface.layers.len();
    let layer_summary = terrain_layers_summary(profile);
    let surface_status = terrain_surface_status(profile);
    let heightmap_status = terrain_heightmap_status(profile);
    let terrain_package_status = terrain_package_status(profile);
    let target_resident_chunks = resident_chunks(profile.terrain.streaming.chunk_radius);
    let launch_radius = launch_blocking_warm_radius(profile.terrain.streaming.chunk_radius);
    let launch_resident_chunks = resident_chunks(launch_radius);
    newengine_ulog_api::ulog::info!(
        "authored-world ymap read: profile materialized path='{}' source='{}' title='{}' terrain_cells={}x{} terrain_size={}x{} base_height={} height_scale={} generator='{}' surface_mode='multi_textured_projected_3ch' surface_status='{}' surface_layer_count={} projected_surface=[forest='{}',sand='{}',rock='{}'] surface_layers=[{}] heightmap_status='{}' heightmap_enabled={} heightmap_source='{}' heightmap_mode='{}' heightmap_strength={} heightmap_range=[{},{}] heightmap_tile_scale=[{},{}] heightmap_tile_offset=[{},{}] heightmap_invert={} streaming_enabled={} chunk_radius={} unload_radius={} max_chunks_per_frame={} launch_radius={} launch_resident_chunks={} target_resident_chunks={} definitions={} prefabs={} terrain_package_status='{}'",
        logical_path,
        source_label,
        profile.title,
        profile.terrain.cells_x,
        profile.terrain.cells_z,
        profile.terrain.size_x,
        profile.terrain.size_z,
        profile.terrain.base_height,
        profile.terrain.height_scale,
        profile.terrain.generator.id,
        surface_status,
        surface_layers,
        profile.terrain.surface.forest_base_texture,
        profile.terrain.surface.sand_base_texture,
        profile.terrain.surface.rock_base_texture,
        layer_summary,
        heightmap_status,
        profile.terrain.heightmap.enabled,
        profile.terrain.heightmap.source,
        profile.terrain.heightmap.mode,
        profile.terrain.heightmap.strength,
        profile.terrain.heightmap.min_height,
        profile.terrain.heightmap.max_height,
        profile.terrain.heightmap.tile_scale[0],
        profile.terrain.heightmap.tile_scale[1],
        profile.terrain.heightmap.tile_offset[0],
        profile.terrain.heightmap.tile_offset[1],
        profile.terrain.heightmap.invert,
        profile.terrain.streaming.enabled,
        profile.terrain.streaming.chunk_radius,
        profile.terrain.streaming.unload_radius,
        profile.terrain.streaming.max_chunks_per_frame,
        launch_radius,
        launch_resident_chunks,
        target_resident_chunks,
        profile.definitions.len(),
        profile.prefabs.len(),
        terrain_package_status,
    );

    log_surface_layer_details(logical_path, profile);
    log_heightmap_readiness(logical_path, profile);
    log_streaming_readiness(
        logical_path,
        profile,
        launch_radius,
        launch_resident_chunks,
        target_resident_chunks,
    );
    log_terrain_package_readiness(logical_path, profile, surface_status, heightmap_status);
}

fn log_surface_layer_details(logical_path: &str, profile: &AuthoredWorldProfile) {
    if profile.terrain.surface.layers.is_empty() {
        if profile.terrain.enabled {
            newengine_ulog_api::ulog::warn!(
                "authored-world ymap read: terrain surface package path='{}' status='fallback_single_material' reason='no declarative <surface><layers> entries found'",
                logical_path,
            );
        } else {
            newengine_ulog_api::ulog::info!(
                "authored-world ymap read: terrain surface package path='{}' status='not_applicable' reason='procedural terrain disabled by authored profile'",
                logical_path,
            );
        }
        return;
    }

    for (index, layer) in profile.terrain.surface.layers.iter().enumerate() {
        let projected_slot = projected_surface_slot(layer.role.as_str());
        let projected = match projected_slot {
            "forest_r" => layer.base_texture == profile.terrain.surface.forest_base_texture,
            "sand_g" => layer.base_texture == profile.terrain.surface.sand_base_texture,
            "rock_b" => layer.base_texture == profile.terrain.surface.rock_base_texture,
            _ => false,
        };
        newengine_ulog_api::ulog::info!(
            "authored-world ymap read: terrain surface layer path='{}' index={} role='{}' projected_slot='{}' texture='{}' weight={:.3} uv_scale={:.3} projected={} runtime='TerrainMaterialLayers -> 3-channel terrain shader'",
            logical_path,
            index,
            layer.role,
            projected_slot,
            layer.base_texture,
            layer.weight,
            layer.uv_scale,
            projected,
        );
    }
}

fn log_heightmap_readiness(logical_path: &str, profile: &AuthoredWorldProfile) {
    let heightmap = &profile.terrain.heightmap;
    let status = terrain_heightmap_status(profile);
    newengine_ulog_api::ulog::info!(
        "authored-world ymap read: terrain heightmap path='{}' status='{}' enabled={} source='{}' mode='{}' strength={} range=[{},{}] tile_scale=[{},{}] tile_offset=[{},{}] invert={} runtime_loader='engine.assets.textures.entry_rgba8_v1' policy='heightmap source must be a .ytd@entry texture reference'",
        logical_path,
        status,
        heightmap.enabled,
        heightmap.source,
        heightmap.mode,
        heightmap.strength,
        heightmap.min_height,
        heightmap.max_height,
        heightmap.tile_scale[0],
        heightmap.tile_scale[1],
        heightmap.tile_offset[0],
        heightmap.tile_offset[1],
        heightmap.invert,
    );
}

fn log_streaming_readiness(
    logical_path: &str,
    profile: &AuthoredWorldProfile,
    launch_radius: i32,
    launch_resident_chunks: usize,
    target_resident_chunks: usize,
) {
    newengine_ulog_api::ulog::info!(
        "authored-world ymap read: terrain streaming path='{}' enabled={} render_radius={} unload_radius={} max_chunks_per_frame={} launch_radius={} launch_resident_chunks={} target_resident_chunks={} policy='warm small launch ring before gate; full render radius streams after public Play'",
        logical_path,
        profile.terrain.streaming.enabled,
        profile.terrain.streaming.chunk_radius,
        profile.terrain.streaming.unload_radius,
        profile.terrain.streaming.max_chunks_per_frame,
        launch_radius,
        launch_resident_chunks,
        target_resident_chunks,
    );
}

fn log_terrain_package_readiness(
    logical_path: &str,
    profile: &AuthoredWorldProfile,
    surface_status: &'static str,
    heightmap_status: &'static str,
) {
    let status = terrain_package_status(profile);
    newengine_ulog_api::ulog::info!(
        "authored-world ymap read: terrain package readiness path='{}' status='{}' surface_status='{}' heightmap_status='{}' projected_layers={} required_layers=3 streaming_enabled={} package_contract='ymap.terrain -> surface layers + heightmap + streaming plan'",
        logical_path,
        status,
        surface_status,
        heightmap_status,
        profile.terrain.surface.layers.len(),
        profile.terrain.streaming.enabled,
    );
}

fn terrain_layers_summary(profile: &AuthoredWorldProfile) -> String {
    if profile.terrain.surface.layers.is_empty() {
        return "<none>".to_owned();
    }
    profile
        .terrain
        .surface
        .layers
        .iter()
        .map(|layer| {
            format!(
                "role='{}':texture='{}':weight={:.3}:uv_scale={:.3}",
                layer.role, layer.base_texture, layer.weight, layer.uv_scale
            )
        })
        .collect::<Vec<_>>()
        .join(";")
}

fn terrain_surface_status(profile: &AuthoredWorldProfile) -> &'static str {
    let surface = &profile.terrain.surface;
    let projected_textures_ready = !surface.forest_base_texture.is_empty()
        && !surface.sand_base_texture.is_empty()
        && !surface.rock_base_texture.is_empty();
    if projected_textures_ready && surface.layers.len() >= 3 {
        "ready_multi_textured_projected_3ch"
    } else if projected_textures_ready {
        "projected_textures_ready_layers_incomplete"
    } else {
        "single_material_fallback"
    }
}

fn terrain_heightmap_status(profile: &AuthoredWorldProfile) -> &'static str {
    let heightmap = &profile.terrain.heightmap;
    if !heightmap.enabled {
        "disabled_by_profile"
    } else if heightmap.source.trim().is_empty() {
        "invalid_empty_source"
    } else if !heightmap.source.to_ascii_lowercase().contains(".ytd@") {
        "invalid_source_not_ytd_entry"
    } else if heightmap.strength.abs() <= f32::EPSILON {
        "zero_strength"
    } else {
        "ready"
    }
}

fn terrain_package_status(profile: &AuthoredWorldProfile) -> &'static str {
    let surface = terrain_surface_status(profile);
    let heightmap = terrain_heightmap_status(profile);
    let surface_ok = matches!(surface, "ready_multi_textured_projected_3ch");
    let heightmap_ok = matches!(heightmap, "ready" | "disabled_by_profile");
    if surface_ok && heightmap_ok {
        "ready"
    } else if surface_ok {
        "surface_ready_heightmap_degraded"
    } else {
        "degraded"
    }
}

fn projected_surface_slot(role: &str) -> &'static str {
    match role.to_ascii_lowercase().as_str() {
        "forest" | "base" | "grass" => "forest_r",
        "sand" | "path" | "soil" => "sand_g",
        "rock" | "slope" | "stone" => "rock_b",
        _ => "aux_unprojected",
    }
}

fn resident_chunks(radius: i32) -> usize {
    let radius = radius.max(0) as usize;
    let width = radius.saturating_mul(2).saturating_add(1);
    width.saturating_mul(width)
}

fn launch_blocking_warm_radius(target_radius: i32) -> i32 {
    let default_launch_warm_radius = newengine_game_data::default_game_data()
        .world
        .terrain
        .streaming
        .launch_warm_radius;
    let requested = crate::env_config::var_i32(
        "NEWENGINE_SCENE_TERRAIN_LAUNCH_WARM_RADIUS",
        default_launch_warm_radius,
        i32::MIN,
        i32::MAX,
    );
    requested.clamp(0, target_radius.max(0))
}

fn ymap_payload_section_label(value: &Value) -> &'static str {
    if value.pointer("/map/profile").is_some() {
        "map.profile"
    } else if value.get("profile").is_some() {
        "profile"
    } else if value.get("payload").is_some() {
        "payload"
    } else if value.get("scene").is_some() {
        "scene_rejected"
    } else {
        "root"
    }
}

fn count_named_arrays(value: &Value, names: &[&str]) -> usize {
    match value {
        Value::Array(items) => items
            .iter()
            .map(|item| count_named_arrays(item, names))
            .sum(),
        Value::Object(map) => map
            .iter()
            .map(|(key, value)| {
                let own = if names.iter().any(|name| key.eq_ignore_ascii_case(name)) {
                    value.as_array().map(Vec::len).unwrap_or(0)
                } else {
                    0
                };
                own + count_named_arrays(value, names)
            })
            .sum(),
        _ => 0,
    }
}

fn count_named_objects(value: &Value, names: &[&str]) -> usize {
    match value {
        Value::Array(items) => items
            .iter()
            .map(|item| count_named_objects(item, names))
            .sum(),
        Value::Object(map) => map
            .iter()
            .map(|(key, value)| {
                let own = if names.iter().any(|name| key.eq_ignore_ascii_case(name))
                    && value.is_object()
                {
                    1
                } else {
                    0
                };
                own + count_named_objects(value, names)
            })
            .sum(),
        _ => 0,
    }
}

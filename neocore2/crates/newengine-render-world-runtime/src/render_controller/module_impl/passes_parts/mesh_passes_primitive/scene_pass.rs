use super::*;
use crate::render_controller::module_impl::passes::mesh_visibility::sphere_screen_coverage_hint;
use newengine_math::collections::FxHashSet;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum PrimitivePassSlice {
    All,
    NonDecal,
    DecalOnly,
}

impl PrimitivePassSlice {
    #[inline]
    fn accepts(self, decal: bool) -> bool {
        match self {
            Self::All => true,
            Self::NonDecal => !decal,
            Self::DecalOnly => decal,
        }
    }
}

pub(super) fn draw_primitives_for_pass(
    this: &mut RuntimeRenderController,
    r: &mut dyn newengine_core::render::RenderApi,
    scene: &newengine_scene::Scene,
    lit: newengine_material_domain_api::LitPipeline,
    pass: SceneMeshPass,
    viewproj: Mat4,
    lights: &PackedLights,
    shadow_texture: TextureId,
    local_shadow_texture: TextureId,
    runtime: bool,
    camera_position: Vec3,
    _camera_forward: Vec3,
    deferred: bool,
    slice: PrimitivePassSlice,
) -> newengine_core::EngineResult<()> {
    let stage_profile = runtime
        && (this.frame.frame_index <= 3 || this.frame.frame_index.is_multiple_of(30))
        && newengine_runtime_policy::render_runtime_policy().primitive_stage_log;
    let stage_total_started = stage_profile.then(std::time::Instant::now);
    let scan_started = stage_profile.then(std::time::Instant::now);
    let (primitive_snapshot, snapshot_reused) = this.primitive_scene_snapshot(scene, runtime);
    if runtime
        && matches!(pass, SceneMeshPass::GBuffer)
        && std::env::var_os("NEWENGINE_PRIMITIVE_ENTRY_TRACE").is_some()
        && (this.frame.frame_index <= 3 || this.frame.frame_index.is_multiple_of(30))
    {
        eprintln!(
            "NSDIAG primitive-snapshot frame={} camera=({:.3},{:.3},{:.3}) entries={} queried={}",
            this.frame.frame_index,
            camera_position.x,
            camera_position.y,
            camera_position.z,
            primitive_snapshot.entries.len(),
            primitive_snapshot.queried_count
        );
        for source in primitive_snapshot.entries.iter() {
            let cols = source.render_model.to_cols_array();
            let bounds = source
                .local_bounds
                .map(|(center, radius)| {
                    format!(
                        "local_center=({:.3},{:.3},{:.3}) radius={:.3}",
                        center.x, center.y, center.z, radius
                    )
                })
                .unwrap_or_else(|| "local_bounds=<none>".to_owned());
            eprintln!(
                "NSDIAG primitive-entry entity={} primitive={} origin=({:.3},{:.3},{:.3}) {} role={:?} material={:?}",
                source.entity_key, source.primitive.id.0, cols[12], cols[13], cols[14], bounds,
                source.render_options.role, source.material_ref
            );
        }
    }
    let reg_lock = this.bridges.scene.primitives();
    let reg = reg_lock.read();
    let mats_lock = this.bridges.scene.materials();
    let mats = mats_lock.read();
    let visibility_settings = primitive_visibility_settings(runtime, viewproj);

    type PrimitiveDrawEntry = (
        f32,
        u64,
        Primitive,
        Mat4,
        Option<newengine_materials::MaterialRef>,
        u8,
        Option<newengine_model_domain_api::FoliageInstanceRuntime>,
        Option<EnvironmentDomeRenderState>,
        f32,
    );

    let mut sky_entries: Vec<PrimitiveDrawEntry> = Vec::new();
    let mut foliage_entries: Vec<PrimitiveDrawEntry> = Vec::new();
    let mut entries: Vec<PrimitiveDrawEntry> = Vec::new();
    let mut sky_seen = 0usize;
    let mut sky_profile_culled = 0usize;
    let mut visibility_feedback_culled = 0usize;
    let mut gpu_indirect_migrated = 0usize;
    for source in primitive_snapshot.entries.iter() {
        if pass.is_gbuffer() && this.gpu_indirect_entity_migrated(source.entity_key) {
            gpu_indirect_migrated = gpu_indirect_migrated.saturating_add(1);
            continue;
        }
        let prim = source.primitive;
        let render_model = source.render_model;
        let sky_dome_runtime = source.environment_dome.as_ref();
        let asset_label = sky_dome_runtime
            .and_then(|sky| sky.asset_ref.as_deref())
            .unwrap_or("primitive://runtime");
        let definition_label = sky_dome_runtime
            .and_then(|sky| sky.definition_ref.as_deref())
            .unwrap_or("<none>");
        let mesh_render_options = &source.render_options;
        let mut draw_flags = primitive_draw_flags(mesh_render_options);
        if source.authored_pbr_required {
            draw_flags |= PRIMITIVE_DRAW_AUTHORED_BASE_REQUIRED;
        }
        let decal_role = has_primitive_flag(draw_flags, PRIMITIVE_DRAW_DECAL_ROLE);
        if !slice.accepts(decal_role) {
            continue;
        }
        let follows_view = has_primitive_flag(draw_flags, PRIMITIVE_DRAW_FOLLOW_VIEW);
        let sky_role = has_primitive_flag(draw_flags, PRIMITIVE_DRAW_SKY_ROLE);
        let background_sky = has_primitive_flag(draw_flags, PRIMITIVE_DRAW_SKY_BACKGROUND);
        if sky_role {
            sky_seen += 1;
        }
        let role_cull_reason = primitive_role_cull_reason(
            mesh_render_options,
            pass,
            this.runtime_profile().draw_sky_visuals(),
            deferred,
        );
        if role_cull_reason == Some("opaque_role_routed_to_deferred_gbuffer") {
            draw_flags |= PRIMITIVE_DRAW_DEFERRED_OPAQUE_ROUTE;
        }
        if let Some(culled_reason) = role_cull_reason {
            // The generic deferred role gate cannot classify material alpha yet.
            // Delay only its opaque-forward decision until the resolved material is
            // known; all structural role culls remain immediate.
            if culled_reason != "opaque_role_routed_to_deferred_gbuffer" {
                if sky_role {
                    sky_profile_culled += 1;
                }
                if runtime && (route_diagnostics_due(this.frame.frame_index)) {
                    newengine_ulog_api::ulog::debug!(
                        "mesh.role.route: definition='{}' asset='{}' role={:?} transform_policy={:?} pass='{}' emitted=false culled_reason='{}' asset_source='engine.assets.definitions'",
                        definition_label,
                        asset_label,
                        mesh_render_options.role,
                        mesh_render_options.transform_policy,
                        pass.label(),
                        culled_reason
                    );
                }
                if background_sky && this.frame.frame_index <= 2 {
                    newengine_ulog_api::ulog::info!(
                        "sky.draw_list: authored background dome skipped policy='mesh-render-role-routing' role={:?} reason='{}'",
                        mesh_render_options.role,
                        culled_reason
                    );
                }
                continue;
            }
        }
        if has_primitive_flag(draw_flags, PRIMITIVE_DRAW_FOLIAGE_ROLE) {
            if let Some(foliage) = source.foliage_runtime {
                let distance = distance_sq_to_camera(render_model, camera_position).sqrt();
                if !foliage.is_visible(distance, false) {
                    continue;
                }
            }
        }
        let transformed_bounds = source.world_bounds;
        if runtime && !follows_view && !sky_role {
            if let Some((center_ws, radius_ws)) = transformed_bounds {
                if !sphere_within_render_distance(
                    camera_position,
                    center_ws,
                    radius_ws,
                    visibility_settings.max_distance,
                ) {
                    continue;
                }
                if visibility_settings.culling_enabled
                    && !frustum_sphere_visible(
                        &visibility_settings.frustum,
                        camera_position,
                        center_ws,
                        radius_ws,
                        visibility_settings.max_distance,
                        visibility_settings.near_accept_distance,
                    )
                {
                    continue;
                }
            } else if distance_sq_to_camera(render_model, camera_position)
                > visibility_settings.max_distance * visibility_settings.max_distance
            {
                continue;
            }
        }
        let distance_sq = if follows_view || sky_role {
            0.0
        } else {
            distance_sq_to_camera(render_model, camera_position)
        };
        let screen_coverage = if follows_view || sky_role {
            1.0
        } else if let Some((center_ws, radius_ws)) = transformed_bounds {
            sphere_screen_coverage_hint(radius_ws, (center_ws - camera_position).length())
        } else {
            sphere_screen_coverage_hint(1.0, distance_sq.sqrt())
        };
        if runtime
            && !follows_view
            && !sky_role
            && this.visibility_should_cull_world_primitive(
                source.entity_key,
                distance_sq.sqrt(),
                screen_coverage,
            )
        {
            visibility_feedback_culled = visibility_feedback_culled.saturating_add(1);
            continue;
        }
        let entry = (
            distance_sq,
            source.entity_key,
            prim,
            render_model,
            source.material_ref,
            draw_flags,
            source.foliage_runtime,
            source.environment_dome.clone(),
            screen_coverage,
        );
        if sky_role {
            sky_entries.push(entry);
        } else if has_primitive_flag(draw_flags, PRIMITIVE_DRAW_FOLIAGE_ROLE) {
            foliage_entries.push(entry);
        } else {
            entries.push(entry);
        }
    }
    if runtime
        && (this.frame.frame_index <= 3 || route_diagnostics_due(this.frame.frame_index))
    {
        newengine_ulog_api::ulog::debug!(
            "render.visibility.effectiveness: frame={} pass='{}' snapshot={} eligible={} queried={} confirmed_occluded={} actually_culled={} gpu_indirect_migrated={}",
            this.frame.frame_index,
            pass.label(),
            this.frame.visibility.last_source_count,
            this.frame.visibility.last_eligible_count,
            this.frame.visibility.last_candidate_count,
            this.frame
                .visibility
                .history
                .values()
                .filter(|history| history.confirmed_occluded)
                .count(),
            visibility_feedback_culled,
            gpu_indirect_migrated,
        );
    }
    sort_by_distance_then_key(&mut sky_entries);
    sort_and_truncate_by_distance_then_key(
        &mut foliage_entries,
        foliage_instance_budget(runtime, false),
    );
    sort_and_truncate_by_distance_then_key(&mut entries, primitive_budget(runtime, false));
    let scan_ms = scan_started
        .map(|started| started.elapsed().as_secs_f64() * 1000.0)
        .unwrap_or(0.0);
    let plan_started = stage_profile.then(std::time::Instant::now);
    if runtime && (route_diagnostics_due(this.frame.frame_index)) {
        newengine_ulog_api::ulog::debug!(
            "sky.draw_list: seen={} emitted={} profile_culled={} visibility_feedback_culled={} pass='viewport_forward' depth_write=false shadow=false route='mesh_render_options' opaque_candidates={} opaque_budget={} draw_sky_visuals={}",
            sky_seen,
            sky_entries.len(),
            sky_profile_culled,
            visibility_feedback_culled,
            entries.len(),
            primitive_budget(runtime, false),
            this.runtime_profile().draw_sky_visuals()
        );
    }
    let plan_cache_capacity = sky_entries
        .len()
        .saturating_add(foliage_entries.len())
        .saturating_add(entries.len());
    let mut plan_cache: FxHashMap<PrimitivePlanKey, PrimitiveGpuPlan> =
        FxHashMap::with_capacity_and_hasher(plan_cache_capacity, Default::default());
    let mut written_ubos = FxHashSet::<u64>::default();
    let mut sky_background_batches = InstanceBatchSet::default();
    let mut sky_foreground_batches = InstanceBatchSet::default();
    let mut foliage_batches = InstanceBatchSet::default();
    let mut opaque_batches = InstanceBatchSet::default();

    // Keep sky in ordered replay buckets. `InstanceBatchSet` sorts by pipeline / bind
    // group / mesh for performance, so the dome must not share the same unordered
    // set with sun/moon discs: draw authored dome first, sky foreground discs next,
    // then world opaque batches.
    for (
        distance_sq,
        _entity_key,
        prim,
        model,
        material_ref,
        draw_flags,
        foliage_runtime,
        sky_runtime,
        screen_coverage,
    ) in sky_entries
        .into_iter()
        .chain(foliage_entries)
        .chain(entries)
    {
        let follows_view = has_primitive_flag(draw_flags, PRIMITIVE_DRAW_FOLLOW_VIEW);
        let foliage_role = has_primitive_flag(draw_flags, PRIMITIVE_DRAW_FOLIAGE_ROLE);
        let decal_role = has_primitive_flag(draw_flags, PRIMITIVE_DRAW_DECAL_ROLE);
        let sky_role = has_primitive_flag(draw_flags, PRIMITIVE_DRAW_SKY_ROLE);
        let background_sky = has_primitive_flag(draw_flags, PRIMITIVE_DRAW_SKY_BACKGROUND);
        let receive_shadows = has_primitive_flag(draw_flags, PRIMITIVE_DRAW_RECEIVE_SHADOWS);
        let authored_pbr_required =
            has_primitive_flag(draw_flags, PRIMITIVE_DRAW_AUTHORED_BASE_REQUIRED);
        let model = if follows_view {
            recenter_model_translation(model, camera_position)
        } else {
            model
        };
        if pass.is_gbuffer() && sky_role {
            continue;
        }
        let plan_key =
            PrimitivePlanKey::new(prim, material_ref, sky_role, decal_role, pass.is_gbuffer());
        let plan = if let Some(plan) = plan_cache.get(&plan_key).copied() {
            plan
        } else {
            let gpu = ensure_primitive_gpu(&reg, prim.id, &mut this.gpu.meshes.prim_cache, r)?;
            let owned_material_plan =
                this.gpu
                    .material
                    .resolved_lit_plans
                    .resolve(&*mats, material_ref, prim.color);
            let mut material_plan = owned_material_plan.as_borrowed();
            let forward_only_alpha_surface =
                material_plan.alpha_blend || material_plan.alpha_cutoff > 0.0;
            // Deferred is currently an opaque-only material path. Masked surfaces
            // need their authored cutoff and alpha-blended surfaces need forward
            // composition; never reinterpret arbitrary RGBA texture alpha in GBuffer.
            if pass.is_gbuffer() && forward_only_alpha_surface {
                continue;
            }
            if has_primitive_flag(draw_flags, PRIMITIVE_DRAW_DEFERRED_OPAQUE_ROUTE)
                && !forward_only_alpha_surface
            {
                continue;
            }
            if let Some(runtime) = sky_runtime.as_ref() {
                material_plan.uv_transform = runtime.uv_transform;
                material_plan.material_params = runtime.material_params;
                material_plan.emissive_radiance = runtime.emissive_params;
            }

            if authored_pbr_required && material_plan.base_color_texture.is_none() {
                // An authored weapon/world-item surface without its base texture is incomplete.
                // Do not expose the generic white texture as a successful production material.
                continue;
            }
            if !sky_role {
                this.request_material_set_with_view_hints(
                    material_plan.base_color_texture,
                    material_plan.normal_texture,
                    material_plan.roughness_texture,
                    screen_coverage,
                    distance_sq.sqrt(),
                    if authored_pbr_required { u8::MAX } else { 0 },
                    authored_pbr_required,
                );
            }
            let base_tex = if let Some(path) = material_plan.base_color_texture {
                if foliage_role
                    || material_plan.alpha_cutoff > 0.0
                    || material_plan.alpha_blend
                    || authored_pbr_required
                {
                    let status_owner = if authored_pbr_required {
                        "render.authored_pbr_object"
                    } else if material_plan.alpha_blend {
                        "render.world_transparent"
                    } else {
                        "render.world_foliage"
                    };
                    let Some(texture) = this.material_texture_if_ready(r, path, status_owner)
                    else {
                        // Never expose the generic white fallback through leaf/grass alpha
                        // cards or an authored PBR object with authored albedo. For foliage it turns
                        // transparent atlas texels opaque; for weapons/world items it produces a
                        // conspicuous solid-white model while the real YTD is loading or failed.
                        continue;
                    };
                    texture
                } else {
                    this.material_texture_or_default(r, Some(path), lit.white_texture)
                }
            } else {
                lit.white_texture
            };
            let normal_tex = if authored_pbr_required {
                match material_plan.normal_texture {
                    Some(path) => {
                        let Some(texture) = this.material_texture_if_ready(
                            r,
                            path,
                            "render.equipped_weapon.normal",
                        ) else {
                            continue;
                        };
                        texture
                    }
                    None => continue,
                }
            } else {
                this.material_texture_or_default(
                    r,
                    material_plan.normal_texture,
                    lit.flat_normal_texture,
                )
            };
            let roughness_tex = if authored_pbr_required {
                match material_plan.roughness_texture {
                    Some(path) => {
                        let Some(texture) = this.material_texture_if_ready(
                            r,
                            path,
                            "render.equipped_weapon.roughness",
                        ) else {
                            continue;
                        };
                        texture
                    }
                    None => continue,
                }
            } else {
                this.material_texture_or_default(
                    r,
                    material_plan.roughness_texture,
                    lit.white_texture,
                )
            };
            let speedtree_authored_texture = material_plan
                .base_color_texture
                .map(|path| {
                    path.contains("/foliage/speedtree/") || path.contains("\\foliage\\speedtree\\")
                })
                .unwrap_or(false);
            let sampler = if sky_role {
                // Procedural SkyDome noise is tiled in a projected cloud plane.
                lit.repeat_sampler
            } else if speedtree_authored_texture {
                // SpeedTree generated/source UVs intentionally cross the 0..1 boundary
                // on bark and some leaf/cluster cards. Clamp smears the outer texel
                // across those triangles and exposes the card plane.
                lit.repeat_sampler
            } else if material_plan.alpha_cutoff > 0.0 {
                // Generic packed alpha atlases keep clamp semantics.
                lit.clamp_sampler
            } else if material_plan.has_textures() {
                lit.repeat_sampler
            } else {
                lit.clamp_sampler
            };
            let material_shadow_texture = if !receive_shadows || !material_plan.receive_shadows {
                lit.white_texture
            } else {
                shadow_texture
            };
            let material_local_shadow_texture =
                if pass.is_gbuffer() || !receive_shadows || !material_plan.receive_shadows {
                    lit.white_texture
                } else {
                    local_shadow_texture
                };
            let pipeline = if pass.is_gbuffer() {
                if material_plan.double_sided {
                    lit.gbuffer_instanced_double_sided_pipeline
                } else {
                    lit.gbuffer_instanced_pipeline
                }
            } else if sky_role {
                lit.sky_instanced_pipeline
            } else if decal_role && material_plan.double_sided {
                lit.decal_instanced_double_sided_pipeline
            } else if decal_role {
                lit.decal_instanced_pipeline
            } else if material_plan.alpha_blend && material_plan.double_sided {
                lit.instanced_alpha_double_sided_pipeline
            } else if material_plan.alpha_blend {
                lit.instanced_alpha_pipeline
            } else if material_plan.double_sided {
                lit.instanced_double_sided_pipeline
            } else {
                lit.instanced_pipeline
            };
            let mesh_key = prim.id.0;
            let ubo_key = instance_batch_ubo_key(
                0x1b17_f011_0000_0000,
                pipeline,
                base_tex,
                normal_tex,
                roughness_tex,
                material_shadow_texture,
                material_local_shadow_texture,
                sampler,
            );

            let per = this.ensure_per_draw_ubo_with_binding(
                r,
                lit,
                ubo_key,
                base_tex,
                normal_tex,
                roughness_tex,
                material_shadow_texture,
                material_local_shadow_texture,
                sampler,
            )?;

            // Instance transforms and material scalars live in the instance
            // buffer. The UBO depends only on the pipeline texture set and frame
            // lighting, so different meshes can share it safely. Update each
            // shared UBO once per pass/frame instead of once per mesh plan.
            if written_ubos.insert(ubo_key) {
                crate::render_controller::module_impl::passes_ubo::write_lit_ubo_ex(
                    r,
                    per.ubo,
                    Mat4::IDENTITY,
                    Mat4::IDENTITY,
                    [1.0, 1.0, 1.0, 1.0],
                    [0.0, 0.0, 0.0],
                    0.0,
                    [1.0, 1.0, 0.0, 0.0],
                    [1.0, 0.75, 0.0, 1.0],
                    lights,
                )?;
            }

            let plan = PrimitiveGpuPlan {
                gpu,
                pipeline,
                bind_group: per.bg,
                base_texture: base_tex,
                normal_texture: normal_tex,
                roughness_texture: roughness_tex,
                shadow_texture: material_shadow_texture,
                sampler,
                mesh_key,
                base_color: material_plan.base_color,
                emissive_radiance: material_plan.emissive_radiance,
                alpha_cutoff: material_plan.alpha_cutoff,
                uv_transform: material_plan.uv_transform,
                material_params: material_plan.material_params,
            };
            plan_cache.insert(plan_key, plan);
            plan
        };

        if runtime
            && matches!(pass, SceneMeshPass::Forward)
            && lights.shadow_view_forward[3] >= 6.5
            && route_diagnostics_due(this.frame.frame_index)
        {
            let cols = model.to_cols_array();
            newengine_ulog_api::ulog::debug!(
                "receiver diagnostic instance: primitive_id={} token24={} origin=({:.3},{:.3},{:.3}) base=({:.3},{:.3},{:.3},{:.3}) material_params=({:.3},{:.3},{:.3},{:.3})",
                prim.id.0,
                diagnostic_instance_token(prim.id.0),
                cols[12], cols[13], cols[14],
                plan.base_color[0], plan.base_color[1], plan.base_color[2], plan.base_color[3],
                plan.material_params[0], plan.material_params[1], plan.material_params[2], plan.material_params[3],
            );
        }

        let instance = RenderInstanceRaw::new(
            model,
            viewproj * model,
            plan.base_color,
            plan.uv_transform,
            plan.material_params,
            plan.emissive_radiance,
            plan.alpha_cutoff,
            prim.id.0,
        );
        let instance = if foliage_role {
            let wind = foliage_runtime.unwrap_or_default();
            instance.with_foliage_wind(wind.wind_enabled, wind.wind_direction, wind.wind_strength)
        } else {
            instance
        };
        let batch_key = InstanceBatchKey::new(
            plan.pipeline,
            plan.bind_group,
            plan.gpu,
            plan.base_texture,
            plan.normal_texture,
            plan.roughness_texture,
            plan.shadow_texture,
            plan.sampler,
            plan.mesh_key,
        );
        if sky_role && background_sky {
            sky_background_batches.push(
                batch_key,
                plan.pipeline,
                plan.bind_group,
                plan.gpu,
                instance,
            );
        } else if sky_role {
            sky_foreground_batches.push(
                batch_key,
                plan.pipeline,
                plan.bind_group,
                plan.gpu,
                instance,
            );
        } else if foliage_role {
            foliage_batches.push(
                batch_key,
                plan.pipeline,
                plan.bind_group,
                plan.gpu,
                instance,
            );
        } else {
            opaque_batches.push(
                batch_key,
                plan.pipeline,
                plan.bind_group,
                plan.gpu,
                instance,
            );
        }
        this.diagnostics
            .overlay_metrics
            .record_indexed_triangles(plan.gpu.index_count);
    }

    if runtime && route_diagnostics_due(this.frame.frame_index) {
        newengine_ulog_api::ulog::debug!(
            "primitive.batch.plan: pass='{}' plans={} shared_ubos={} batches=[sky_bg:{},sky_fg:{},foliage:{},opaque:{}] instances=[sky_bg:{},sky_fg:{},foliage:{},opaque:{}] policy='UBO keyed by pipeline texture set; mesh transform/material scalars stay in instance data'",
            pass.label(),
            plan_cache.len(),
            written_ubos.len(),
            sky_background_batches.batch_count(),
            sky_foreground_batches.batch_count(),
            foliage_batches.batch_count(),
            opaque_batches.batch_count(),
            sky_background_batches.instance_count(),
            sky_foreground_batches.instance_count(),
            foliage_batches.instance_count(),
            opaque_batches.instance_count(),
        );
    }

    let foliage_batch_count = foliage_batches.batch_count();
    let foliage_instance_count = foliage_batches.instance_count();
    if runtime
        && foliage_instance_count > 0
        && (this.frame.frame_index <= 3 || route_diagnostics_due(this.frame.frame_index))
    {
        newengine_ulog_api::ulog::info!(
            "foliage.instance_batch: frame={} pass='{}' gpu_batches={} instances={} policy='MeshRenderRole::FoliageInstanced -> shared source mesh + hardware instance buffer'",
            this.frame.frame_index,
            pass.label(),
            foliage_batch_count,
            foliage_instance_count,
        );
    }

    if sky_background_batches.is_empty()
        && sky_foreground_batches.is_empty()
        && foliage_batches.is_empty()
        && opaque_batches.is_empty()
    {
        return Ok(());
    }

    let plan_ms = plan_started
        .map(|started| started.elapsed().as_secs_f64() * 1000.0)
        .unwrap_or(0.0);
    let upload_started = stage_profile.then(std::time::Instant::now);
    let ordered_batches = sky_background_batches
        .into_sorted_batches()
        .into_iter()
        .chain(sky_foreground_batches.into_sorted_batches())
        .chain(foliage_batches.into_sorted_batches())
        .chain(opaque_batches.into_sorted_batches())
        .collect::<Vec<_>>();
    let packed_upload = this
        .gpu
        .meshes
        .instance_uploader
        .upload_batches(r, &ordered_batches)?;
    let upload_ms = upload_started
        .map(|started| started.elapsed().as_secs_f64() * 1000.0)
        .unwrap_or(0.0);
    let replay_started = stage_profile.then(std::time::Instant::now);

    let mut replay = InstancedReplayState::default();
    for (batch, instance_slice) in ordered_batches
        .into_iter()
        .zip(packed_upload.slices.iter().copied())
    {
        let instance_count = batch.instances.len() as u32;
        replay.set_pipeline(r, batch.pipeline)?;
        replay.set_bind_group0(r, batch.bind_group)?;
        replay.set_vertex_buffer(r, 0, BufferSlice::new(batch.gpu.vb, 0))?;
        replay.set_vertex_buffer(r, 1, instance_slice)?;
        replay.set_index_buffer(r, BufferSlice::new(batch.gpu.ib, 0), IndexFormat::U32)?;
        r.draw_indexed(draw_indexed_instanced_args(
            batch.gpu.index_count,
            instance_count,
        ))?;
    }

    let replay_ms = replay_started
        .map(|started| started.elapsed().as_secs_f64() * 1000.0)
        .unwrap_or(0.0);
    if stage_profile {
        let total_ms = stage_total_started
            .map(|started| started.elapsed().as_secs_f64() * 1000.0)
            .unwrap_or(0.0);
        let material_plan_cache = this.gpu.material.resolved_lit_plans.stats();
        newengine_ulog_api::ulog::info!(
            "primitive.stage.profile: frame={} pass='{}' total_ms={:.3} scan_ms={:.3} snapshot_reused={} snapshot_entries={} queried={} visibility_feedback_culled={} material_plan_entries={} material_plan_hits={} material_plan_misses={} material_plan_negative_hits={} material_plan_invalidations={} plan_batch_ms={:.3} upload_ms={:.3} replay_ms={:.3} batches={} instances={} bytes={}",
            this.frame.frame_index,
            pass.label(),
            total_ms,
            scan_ms,
            snapshot_reused,
            primitive_snapshot.entries.len(),
            primitive_snapshot.queried_count,
            visibility_feedback_culled,
            material_plan_cache.entries,
            material_plan_cache.hits,
            material_plan_cache.misses,
            material_plan_cache.negative_hits,
            material_plan_cache.invalidations,
            plan_ms,
            upload_ms,
            replay_ms,
            packed_upload.slices.len(),
            packed_upload.instance_count,
            packed_upload.bytes_written,
        );
    }

    if runtime && route_diagnostics_due(this.frame.frame_index) {
        newengine_ulog_api::ulog::debug!(
            "primitive.instance_upload: pass='{}' writes=1 batches={} instances={} bytes={} policy='single packed write per pass'",
            pass.label(),
            packed_upload.slices.len(),
            packed_upload.instance_count,
            packed_upload.bytes_written,
        );
    }

    Ok(())
}

include!("scene_pass/tests.rs");

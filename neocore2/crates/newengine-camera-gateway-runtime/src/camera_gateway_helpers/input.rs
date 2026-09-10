#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct RoutedPlayerInput {
    pub(super) move_mask: u64,
    pub(super) look_delta: Vec2,
    pub(super) look_active: bool,
}

#[inline]
pub(super) fn route_player_input_channels(
    input: &CameraGatewayInput,
    gameplay_capture: newengine_input_capture_api::GameplayInputCapture,
) -> RoutedPlayerInput {
    let movement_blocked = input.gameplay_movement_gated || gameplay_capture.block_player_movement;
    let look_blocked = input.camera_navigation_gated || gameplay_capture.block_camera_navigation;
    let dx_px = if input.dx_px.is_finite() {
        input.dx_px.clamp(-120.0, 120.0)
    } else {
        0.0
    };
    let dy_px = if input.dy_px.is_finite() {
        input.dy_px.clamp(-120.0, 120.0)
    } else {
        0.0
    };
    let raw_look_active = dx_px.abs() > f32::EPSILON || dy_px.abs() > f32::EPSILON;
    RoutedPlayerInput {
        move_mask: if movement_blocked { 0 } else { input.move_mask },
        look_delta: if look_blocked {
            Vec2::ZERO
        } else {
            // Captured-cursor backends can occasionally report warp/recenter spikes. Gameplay
            // look is render-cadence direct, especially Orbit, so bound the packet before it can
            // become a multi-radian angular jump. Generic nav already applies the same policy.
            Vec2::new(-dx_px, -dy_px)
        },
        // A real mouse packet is sufficient evidence that gameplay look is active.
        // The legacy `active` bit comes from viewport/UI routing and can legitimately lag
        // one frame behind raw DeviceEvent motion, which previously dropped pure pitch input.
        look_active: !look_blocked && (input.active || raw_look_active),
    }
}

#[inline]
fn aabb_distance_sq_to_point(min: Vec3, max: Vec3, point: Vec3) -> f32 {
    let nearest = point.clamp(min, max);
    (point - nearest).length_squared()
}

/// Camera collision uses the same ECS-change contract as the physics bridge: static authored
/// meshes are structural data and must not be rescanned on every render frame. The cache keeps
/// a slightly wider gather ring than the historical 32 m relevance radius, so the player can move
/// several metres without missing a collider before the next spatial refresh.
#[derive(Clone, Debug)]
struct CameraSpringArmStaticCollisionCache {
    observed_tick: u64,
    center_ws: Vec3,
    initialized: bool,
    rebuild_count: u64,
    meshes: Vec<CameraSpringArmMeshCollider>,
}

impl Default for CameraSpringArmStaticCollisionCache {
    fn default() -> Self {
        Self {
            observed_tick: 0,
            center_ws: Vec3::ZERO,
            initialized: false,
            rebuild_count: 0,
            meshes: Vec::new(),
        }
    }
}

#[inline]
fn static_camera_collision_cache_dirty(
    world: &World,
    cache: &CameraSpringArmStaticCollisionCache,
    center: Vec3,
) -> bool {
    if !cache.initialized || cache.observed_tick == 0 {
        return true;
    }

    // The cached gather ring is 8 m wider than the normal relevance radius. Rescan once the
    // player has consumed that margin so every collider that can enter the old 32 m set was
    // already present in the cached 40 m set.
    const STATIC_RESCAN_DISTANCE: f32 = 8.0;
    if (center - cache.center_ws).length_squared()
        >= STATIC_RESCAN_DISTANCE * STATIC_RESCAN_DISTANCE
    {
        return true;
    }

    let since_tick = cache.observed_tick;
    if world.entities_changed_since(since_tick)
        || world.any_changed_since::<newengine_gameplay_world_runtime::gameplay::StaticMeshCollider>(
            since_tick,
        )
        || world.any_added_since::<newengine_gameplay_world_runtime::gameplay::StaticMeshCollider>(
            since_tick,
        )
    {
        return true;
    }

    world.any_changed_since::<Transform>(since_tick)
        && world.query_changed::<Transform>(since_tick).any(|(entity, _)| {
            world
                .get::<newengine_gameplay_world_runtime::gameplay::StaticMeshCollider>(entity)
                .is_some()
        })
}

/// Projects query-participating gameplay colliders into the camera-runtime neutral
/// spring-arm collision world. This stays backend-neutral and works even when the
/// physics service is between fixed ticks.
pub(super) fn refresh_camera_spring_arm_collision_world(world: &mut World, player: EntityId) {
    let center = newengine_transform::read_entity_world_pose_local_chain(world, player)
        .map(|pose| pose.0)
        .unwrap_or(Vec3::ZERO);
    const DYNAMIC_RELEVANCE_RADIUS: f32 = 32.0;
    const STATIC_GATHER_RADIUS: f32 = 40.0;
    let dynamic_relevance_sq = DYNAMIC_RELEVANCE_RADIUS * DYNAMIC_RELEVANCE_RADIUS;
    let static_gather_sq = STATIC_GATHER_RADIUS * STATIC_GATHER_RADIUS;

    let current_tick = world.tick();
    let mut static_cache = world
        .remove_resource::<CameraSpringArmStaticCollisionCache>()
        .unwrap_or_default();
    let rebuild_static = static_camera_collision_cache_dirty(world, &static_cache, center);

    let mut collision_world = world
        .remove_resource::<CameraSpringArmCollisionWorld>()
        .unwrap_or_default();
    collision_world.clear();

    // Dynamic/query bodies may move between fixed ticks, so keep this small set live every frame.
    for (entity, body) in
        world.query::<newengine_gameplay_world_runtime::gameplay::PhysicsBodyDesc>()
    {
        if body.flags.is_trigger || !body.flags.participates_in_queries {
            continue;
        }
        let Some((position, rotation)) =
            newengine_transform::read_entity_world_pose_local_chain(world, entity)
        else {
            continue;
        };
        let world_from_local = Mat4::from_scale_rotation_translation(Vec3::ONE, rotation, position);
        let bounds = body.shape.local_aabb().transformed(world_from_local);
        if !bounds.min.is_finite()
            || !bounds.max.is_finite()
            || aabb_distance_sq_to_point(bounds.min, bounds.max, center) > dynamic_relevance_sq
        {
            continue;
        }
        collision_world.push_aabb(CameraSpringArmAabbCollider {
            entity,
            min_ws: bounds.min,
            max_ws: bounds.max,
        });
    }

    if rebuild_static {
        static_cache.meshes.clear();
        for (entity, collider) in
            world.query::<newengine_gameplay_world_runtime::gameplay::StaticMeshCollider>()
        {
            let Some((position, rotation)) =
                newengine_transform::read_entity_world_pose_local_chain(world, entity)
            else {
                continue;
            };
            let world_from_local =
                Mat4::from_scale_rotation_translation(Vec3::ONE, rotation, position);
            let bounds = collider.local_bounds.transformed(world_from_local);
            if !bounds.min.is_finite()
                || !bounds.max.is_finite()
                || aabb_distance_sq_to_point(bounds.min, bounds.max, center) > static_gather_sq
            {
                continue;
            }
            static_cache.meshes.push(CameraSpringArmMeshCollider {
                entity,
                revision: collider.revision,
                position_ws: position,
                rotation_ws: rotation.normalize_or_identity(),
                min_ls: collider.local_bounds.min,
                max_ls: collider.local_bounds.max,
                vertices: Arc::clone(&collider.vertices),
                triangles: Arc::clone(&collider.triangles),
            });
        }
        static_cache.center_ws = center;
        static_cache.initialized = true;
        static_cache.rebuild_count = static_cache.rebuild_count.wrapping_add(1);
    }
    static_cache.observed_tick = current_tick;

    // CameraSpringArmCollisionWorld keeps its mesh acceleration cache across clear(), so this is
    // only cheap candidate publication on steady-state frames; BVHs rebuild only on revision change.
    for collider in static_cache.meshes.iter().cloned() {
        collision_world.push_mesh(collider);
    }

    world.insert_resource(static_cache);
    world.insert_resource(collision_world);
}

pub(super) fn apply_runtime_input(
    world: &mut World,
    input: CameraGatewayInput,
    effective_play_mode: GameRunMode,
    service_config: CameraRuntimeServiceConfig,
    frame_index: u64,
) {
    let Some(player) = first_player(world) else {
        return;
    };
    let controller_active = effective_play_mode.wants_direct_player_control()
        && is_player_controller_enabled(world, player);
    let gameplay_capture =
        newengine_gameplay_world_runtime::gameplay::gameplay_input_capture(world);
    let routed = route_player_input_channels(&input, gameplay_capture);
    let command_actions = if controller_active {
        input.gameplay_actions
    } else {
        ActionCommandFrame::default()
    };
    apply_player_command_frame(world, player, frame_index, command_actions);

    if controller_active {
        // Movement and camera look are independent gameplay channels. A dialogue/menu layer
        // may deliberately freeze locomotion while preserving free look; conversely a scripted
        // camera may suppress look without cancelling WASD. Do not collapse either policy into
        // a total player-input gate.
        CameraRuntimeService::apply_player_input(
            world,
            player,
            routed.move_mask,
            routed.look_delta,
            routed.look_active,
            service_config.sprint_multiplier,
            service_config.runner,
            service_config.first_person_body_yaw_limit_radians,
            service_config
                .first_person_body_barrier
                .downward_pitch_limit_radians,
            matches!(
                service_config.runner,
                newengine_camera_runtime::GameplayCameraRunnerKind::FirstPerson
                    | newengine_camera_runtime::GameplayCameraRunnerKind::ThirdPersonAim
            ),
        );
        emit_player_event(
            world,
            player,
            PlayerEventKind::InputApplied,
            "local input sampled",
        );
    } else {
        CameraRuntimeService::clear_player_input(world, player);
    }
}

#[inline]
pub(super) fn camera_nav_input(
    input: CameraGatewayInput,
    play_mode: GameRunMode,
) -> CameraNavInput {
    let mut nav_input = CameraNavInput {
        dx_px: finite_or_zero(input.dx_px).clamp(-240.0, 240.0),
        dy_px: finite_or_zero(input.dy_px).clamp(-240.0, 240.0),
        wheel_y: finite_or_zero(input.wheel_y).clamp(-12.0, 12.0),
        active: input.active,
        look_drag: input.look_drag,
        pan_drag: input.pan_drag,
        ui_busy: input.ui_busy,
        fly_rmb: input.fly_rmb,
        navigation_gated: input.camera_navigation_gated,
        move_mask: input.move_mask,
        speed_scalar: finite_or_one(input.speed_scalar).clamp(0.05, 20.0),
    };
    if play_mode.wants_direct_player_control() {
        nav_input.wheel_y = 0.0;
        nav_input.pan_drag = false;
    }
    if nav_input.navigation_gated {
        nav_input.gate_navigation();
    }
    nav_input
}

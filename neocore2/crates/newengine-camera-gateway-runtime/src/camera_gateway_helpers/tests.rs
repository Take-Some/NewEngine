use super::*;

#[test]
fn movement_gate_preserves_free_look_xy() {
    let input = CameraGatewayInput {
        dx_px: 12.0,
        dy_px: -7.5,
        active: true,
        gameplay_movement_gated: true,
        move_mask: 0x0f,
        ..CameraGatewayInput::default()
    };
    let routed = route_player_input_channels(
        &input,
        newengine_input_capture_api::GameplayInputCapture::none(),
    );
    assert_eq!(routed.move_mask, 0);
    assert_eq!(routed.look_delta, Vec2::new(-12.0, 7.5));
    assert!(routed.look_active);
}

#[test]
fn pure_vertical_mouse_packet_activates_look_even_when_legacy_active_is_stale() {
    let input = CameraGatewayInput {
        dx_px: 0.0,
        dy_px: 9.0,
        active: false,
        move_mask: 0,
        ..CameraGatewayInput::default()
    };
    let routed = route_player_input_channels(
        &input,
        newengine_input_capture_api::GameplayInputCapture::none(),
    );
    assert_eq!(routed.look_delta, Vec2::new(0.0, -9.0));
    assert!(routed.look_active);
}

#[test]
fn gameplay_camera_input_clamps_captured_cursor_warp_spikes() {
    let input = CameraGatewayInput {
        dx_px: 4200.0,
        dy_px: -3600.0,
        active: true,
        ..CameraGatewayInput::default()
    };
    let routed = route_player_input_channels(
        &input,
        newengine_input_capture_api::GameplayInputCapture::none(),
    );
    assert_eq!(routed.look_delta, Vec2::new(-120.0, 120.0));
    assert!(routed.look_active);
}

#[test]
fn camera_gate_blocks_look_without_cancelling_movement() {
    let input = CameraGatewayInput {
        dx_px: 12.0,
        dy_px: -7.5,
        active: true,
        camera_navigation_gated: true,
        move_mask: 0x03,
        ..CameraGatewayInput::default()
    };
    let routed = route_player_input_channels(
        &input,
        newengine_input_capture_api::GameplayInputCapture::none(),
    );
    assert_eq!(routed.move_mask, 0x03);
    assert_eq!(routed.look_delta, Vec2::ZERO);
    assert!(!routed.look_active);
}
#[test]
fn orbit_config_uses_bound_avatar_visual_center_not_capsule_top() {
    use newengine_transform::{set_parent, Transform};

    let mut world = World::new();
    let player = world.spawn();
    let visual_root = world.spawn();
    let _ = world.insert(
        player,
        newengine_gameplay_world_runtime::gameplay::PlayerActor,
    );
    let _ = world.insert(
        player,
        newengine_gameplay_world_runtime::gameplay::PlayerController::local_input(),
    );
    let _ = world.insert(player, CharacterBody::default());
    let _ = world.insert(player, Transform::default());
    let _ = world.insert(
        visual_root,
        Transform {
            // Model root sits on the capsule ground plane plus an authored local offset.
            position: Vec3::new(0.20, -0.80, -0.15),
            rotation: newengine_math::Quat::IDENTITY,
            scale: Vec3::ONE,
        },
    );
    let _ = set_parent(&mut world, visual_root, Some(player));
    let _ = world.insert(
        player,
        newengine_gameplay_world_runtime::gameplay::PlayerModelBinding {
            visual_root: Some(visual_root),
            target_height: 1.80,
            ..Default::default()
        },
    );

    let config = camera_runtime_service_config(&world, CameraViewMode::ThirdPersonOrbit);
    let expected = Vec3::new(0.20, 0.10, -0.15);
    assert!((config.third_person_orbit_pivot_offset_ls - expected).length() < 1.0e-5);
}
#[test]
fn third_person_config_carries_interpolated_player_render_pose() {
    use newengine_transform::Transform;

    let mut world = World::new();
    let player = world.spawn();
    let _ = world.insert(
        player,
        newengine_gameplay_world_runtime::gameplay::PlayerActor,
    );
    let _ = world.insert(
        player,
        newengine_gameplay_world_runtime::gameplay::PlayerController::local_input(),
    );
    let _ = world.insert(player, CharacterBody::default());
    let _ = world.insert(
        player,
        Transform {
            position: Vec3::new(8.0, 0.0, 0.0),
            rotation: newengine_math::Quat::from_rotation_y(1.0),
            scale: Vec3::ONE,
        },
    );
    let render_position = Vec3::new(3.25, 0.0, -1.5);
    let render_rotation = newengine_math::Quat::from_rotation_y(0.35);
    let _ = world.insert(
        player,
        newengine_gameplay_world_runtime::gameplay::PlayerRenderPose {
            position: render_position,
            rotation: render_rotation,
            simulation_position: Vec3::new(8.0, 0.0, 0.0),
            simulation_rotation: newengine_math::Quat::from_rotation_y(1.0),
            fixed_alpha: 0.4,
            source_fixed_tick: 42,
        },
    );

    let config = camera_runtime_service_config(&world, CameraViewMode::ThirdPersonOrbit);
    assert_eq!(
        config.third_person_render_position_ws,
        Some(render_position)
    );
    let configured_rotation = config.third_person_render_rotation_ws.unwrap();
    assert!(configured_rotation.dot(render_rotation).abs() > 0.999999);
}

#[test]
fn static_camera_collision_candidates_are_reused_until_ecs_or_spatial_change() {
    use newengine_gameplay_world_runtime::gameplay::StaticMeshCollider;
    use newengine_transform::Transform;

    let mut world = World::new();
    let player = world.spawn();
    let collider_entity = world.spawn();
    let _ = world.insert(player, Transform::default());
    let _ = world.insert(
        collider_entity,
        Transform {
            position: Vec3::new(0.0, 0.0, 2.0),
            ..Transform::default()
        },
    );
    let _ = world.insert(
        collider_entity,
        StaticMeshCollider::new(
            vec![[-1.0, -1.0, 0.0], [1.0, -1.0, 0.0], [0.0, 1.0, 0.0]],
            vec![[0, 1, 2]],
        )
        .expect("static camera collider"),
    );

    refresh_camera_spring_arm_collision_world(&mut world, player);
    let first_rebuilds = world
        .resource::<CameraSpringArmStaticCollisionCache>()
        .expect("camera static collision cache")
        .rebuild_count;
    assert_eq!(first_rebuilds, 1);
    assert_eq!(
        world
            .resource::<CameraSpringArmCollisionWorld>()
            .expect("camera collision world")
            .telemetry()
            .mesh_count,
        1
    );

    world.advance_tick();
    refresh_camera_spring_arm_collision_world(&mut world, player);
    let cache = world
        .resource::<CameraSpringArmStaticCollisionCache>()
        .expect("camera static collision cache");
    assert_eq!(
        cache.rebuild_count, first_rebuilds,
        "unchanged static geometry must not trigger another O(N) candidate scan"
    );
    assert_eq!(cache.meshes.len(), 1);
}

#[test]
fn static_camera_collision_cache_rebuilds_after_tracked_static_transform_change() {
    use newengine_gameplay_world_runtime::gameplay::StaticMeshCollider;
    use newengine_transform::Transform;

    let mut world = World::new();
    let player = world.spawn();
    let collider_entity = world.spawn();
    let _ = world.insert(player, Transform::default());
    let _ = world.insert(collider_entity, Transform::default());
    let _ = world.insert(
        collider_entity,
        StaticMeshCollider::new(
            vec![[-1.0, -1.0, 2.0], [1.0, -1.0, 2.0], [0.0, 1.0, 2.0]],
            vec![[0, 1, 2]],
        )
        .expect("static camera collider"),
    );

    refresh_camera_spring_arm_collision_world(&mut world, player);
    world.advance_tick();
    world
        .get_mut_tracked::<Transform>(collider_entity)
        .expect("static collider transform")
        .position = Vec3::new(100.0, 0.0, 0.0);
    refresh_camera_spring_arm_collision_world(&mut world, player);

    let cache = world
        .resource::<CameraSpringArmStaticCollisionCache>()
        .expect("camera static collision cache");
    assert_eq!(cache.rebuild_count, 2);
    assert!(cache.meshes.is_empty());
    assert_eq!(
        world
            .resource::<CameraSpringArmCollisionWorld>()
            .expect("camera collision world")
            .telemetry()
            .mesh_count,
        0
    );
}

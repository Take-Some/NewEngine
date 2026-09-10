struct CameraTraceSink {
    writer: BufWriter<File>,
    rows: u64,
}

static CAMERA_TRACE_SINK: OnceLock<Option<StdMutex<CameraTraceSink>>> = OnceLock::new();

fn camera_trace_sink() -> Option<&'static StdMutex<CameraTraceSink>> {
    CAMERA_TRACE_SINK
        .get_or_init(|| {
            let path = newengine_runtime_env::var_os("NEWENGINE_CAMERA_TRACE_FILE")?;
            let file = OpenOptions::new()
                .create(true)
                .truncate(true)
                .write(true)
                .open(path)
                .ok()?;
            let mut writer = BufWriter::new(file);
            let _ = writeln!(
                writer,
                "frame,dt,view,raw_dx,raw_dy,routed_dx,routed_dy,sim_x,sim_y,sim_z,render_x,render_y,render_z,fixed_alpha,fixed_tick,runner,yaw,pitch,anchor_x,anchor_y,anchor_z,pivot_x,pivot_y,pivot_z,desired_x,desired_y,desired_z,collision_target,collision_current,rig_x,rig_y,rig_z,pre_x,pre_y,pre_z,final_x,final_y,final_z,frame_blend,frame_blend_alpha,spheres,aabbs,meshes,cached_meshes,bvh_builds,ctrl_z_start,ctrl_z_after_possess,ctrl_z_before_sync,ctrl_z_after_sync,ctrl_z_after_nav,zoom_z"
            );
            let _ = writer.flush();
            Some(StdMutex::new(CameraTraceSink { writer, rows: 0 }))
        })
        .as_ref()
}

#[allow(clippy::too_many_arguments)]
pub(super) fn trace_gameplay_camera_frame(
    frame_index: u64,
    dt: f32,
    input: &CameraGatewayInput,
    routed: Option<RoutedPlayerInput>,
    active_view: CameraViewMode,
    world: &World,
    player: Option<EntityId>,
    camera: EntityId,
    pre_manager_frame: CameraFrame,
    final_frame: CameraFrame,
    report: Option<&CameraRuntimeOverlayReport>,
    controller_z_phases: [f32; 5],
) {
    let Some(sink) = camera_trace_sink() else {
        return;
    };
    let Ok(mut sink) = sink.lock() else {
        return;
    };

    let nan = f32::NAN;
    let (sim, render, fixed_alpha, fixed_tick) = if let Some(player) = player {
        let sim = newengine_transform::read_entity_world_pose_local_chain(world, player)
            .map(|pose| pose.0)
            .unwrap_or(Vec3::splat(nan));
        let render_pose = world
            .get::<newengine_gameplay_world_runtime::gameplay::PlayerRenderPose>(player)
            .copied();
        let render = render_pose
            .map(|pose| pose.position)
            .unwrap_or(Vec3::splat(nan));
        let alpha = render_pose.map(|pose| pose.fixed_alpha).unwrap_or(nan);
        let tick = render_pose.map(|pose| pose.source_fixed_tick).unwrap_or(0);
        (sim, render, alpha, tick)
    } else {
        (Vec3::splat(nan), Vec3::splat(nan), nan, 0)
    };

    let telemetry = CameraRuntimeService::gameplay_camera_telemetry(world, camera);
    let (
        runner,
        yaw,
        pitch,
        anchor,
        pivot,
        desired,
        collision_target,
        collision_current,
        rig,
        zoom_z,
    ) = if let Some(t) = telemetry {
        (
            format!("{:?}", t.runner),
            t.orbit_yaw,
            t.orbit_pitch,
            t.anchor_ws,
            t.pivot_ws,
            t.desired_camera_ws,
            t.collision_target_distance,
            t.collision_distance,
            t.rig_position_ws,
            t.zoom_z,
        )
    } else {
        (
            "None".to_owned(),
            nan,
            nan,
            Vec3::splat(nan),
            Vec3::splat(nan),
            Vec3::splat(nan),
            nan,
            nan,
            Vec3::splat(nan),
            nan,
        )
    };
    let routed = routed.unwrap_or(RoutedPlayerInput {
        move_mask: 0,
        look_delta: Vec2::ZERO,
        look_active: false,
    });
    let collision = world
        .resource::<CameraSpringArmCollisionWorld>()
        .map(|world| world.telemetry())
        .unwrap_or_default();
    let (frame_blend, frame_blend_alpha) = report
        .map(|report| (report.frame_blend_active, report.frame_blend_alpha))
        .unwrap_or((false, 0.0));

    let _ = writeln!(
        sink.writer,
        "{},{:.9},{:?},{:.4},{:.4},{:.4},{:.4},{:.6},{:.6},{:.6},{:.6},{:.6},{:.6},{:.6},{},{},{:.7},{:.7},{:.6},{:.6},{:.6},{:.6},{:.6},{:.6},{:.6},{:.6},{:.6},{:.6},{:.6},{:.6},{:.6},{:.6},{:.6},{:.6},{:.6},{:.6},{:.6},{:.6},{},{:.6},{},{},{},{},{},{:.6},{:.6},{:.6},{:.6},{:.6},{:.6}",
        frame_index,
        dt,
        active_view,
        input.dx_px,
        input.dy_px,
        routed.look_delta.x,
        routed.look_delta.y,
        sim.x,
        sim.y,
        sim.z,
        render.x,
        render.y,
        render.z,
        fixed_alpha,
        fixed_tick,
        runner,
        yaw,
        pitch,
        anchor.x,
        anchor.y,
        anchor.z,
        pivot.x,
        pivot.y,
        pivot.z,
        desired.x,
        desired.y,
        desired.z,
        collision_target,
        collision_current,
        rig.x,
        rig.y,
        rig.z,
        pre_manager_frame.rig.position.x,
        pre_manager_frame.rig.position.y,
        pre_manager_frame.rig.position.z,
        final_frame.rig.position.x,
        final_frame.rig.position.y,
        final_frame.rig.position.z,
        frame_blend,
        frame_blend_alpha,
        collision.sphere_count,
        collision.aabb_count,
        collision.mesh_count,
        collision.cached_mesh_count,
        collision.accel_builds_this_refresh,
        controller_z_phases[0],
        controller_z_phases[1],
        controller_z_phases[2],
        controller_z_phases[3],
        controller_z_phases[4],
        zoom_z,
    );
    sink.rows = sink.rows.saturating_add(1);
    let _ = sink.writer.flush();
}

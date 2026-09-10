const PLAYER_PALETTE_RUNTIME_VALIDATION_INTERVAL_FRAMES: u64 = 30;

#[inline]
fn should_run_player_palette_runtime_validation(
    player_stable_id: u64,
    frame_index: u64,
    transitioned: bool,
) -> bool {
    transitioned
        || frame_index
            .wrapping_add(player_stable_id)
            .is_multiple_of(PLAYER_PALETTE_RUNTIME_VALIDATION_INTERVAL_FRAMES)
}

#[derive(Clone, Copy, Debug, Default)]
struct PlayerAnimationFinalizeTiming {
    pose_copy_ms: f32,
    look_ms: f32,
    support_ik_ms: f32,
    continuity_eye_ms: f32,
    palette_ms: f32,
    joint_frames_ms: f32,
    braid_ms: f32,
    validation_ms: f32,
    overhead_ms: f32,
}

#[allow(clippy::too_many_arguments)]
fn apply_strict_rifle_tpp_rigid_aim_fallback(
    presentation: &newengine_engine_runtime::gameplay::WeaponPresentationDefinition,
    skeleton: &ModelSkeletonMetadata,
    animation_runtime: &AnimationSkeletonRuntime,
    pose: &mut [JointLocalPose],
    frames: &mut Vec<Mat4>,
    rig: &WeaponArmIkRig,
    aim_alpha: f32,
    view_forward_model: Option<Vec3>,
    aim_controller: &WeaponAimControllerState,
) -> Result<Option<crate::weapon_grip::WeaponRootTransform>, String> {
    // A strict bilateral contract may reject an individual frame when the two authored prop
    // branches drift outside the tiny fusion tolerance. That must disable only independent arm IK,
    // never RMB directional control. Use the firing-side prop as the canonical seed, rotate the
    // whole chest subtree as one rigid system, then republish the weapon from the mutated socket.
    let seed_root = current_bilateral_weapon_root(presentation, rig, frames)
        .or_else(|| {
            rig.right_prop_attachment
                .and_then(|index| frames.get(index).copied())
                .and_then(|frame| {
                    crate::weapon_grip::weapon_root_from_authored_prop_frame(presentation, frame)
                })
        })
        .or_else(|| {
            frames.get(rig.right_palm).copied().and_then(|frame| {
                crate::weapon_grip::weapon_root_from_right_palm(presentation, frame)
            })
        });
    let Some(seed_root) = seed_root else {
        return Ok(None);
    };

    let desired = aim_controller
        .sight_target()
        .or_else(|| (aim_alpha > 1.0e-4).then_some(view_forward_model).flatten());
    let Some(target) = staged_weapon_sight_target(
        crate::weapon_grip::weapon_sight_forward(presentation, seed_root),
        desired,
        aim_alpha,
    ) else {
        return Ok(Some(seed_root));
    };

    let authored_sight = crate::weapon_grip::weapon_sight_forward(presentation, seed_root);
    let _ = apply_native_rifle_upper_body_aim_delta(
        skeleton,
        animation_runtime,
        pose,
        frames,
        rig,
        authored_sight,
        target,
    )?;

    Ok(current_bilateral_weapon_root(presentation, rig, frames)
        .or_else(|| {
            rig.right_prop_attachment
                .and_then(|index| frames.get(index).copied())
                .and_then(|frame| {
                    crate::weapon_grip::weapon_root_from_authored_prop_frame(presentation, frame)
                })
        })
        .or_else(|| {
            frames.get(rig.right_palm).copied().and_then(|frame| {
                crate::weapon_grip::weapon_root_from_right_palm(presentation, frame)
            })
        })
        .or_else(|| {
            crate::weapon_grip::weapon_sight_aligned_root_around_stock_contact(
                presentation,
                seed_root,
                target,
            )
        })
        .or(Some(seed_root)))
}

/// Phase 3: compose the visible pose, solve authored IK, enforce continuity/eye invariants,
/// build the skin palette, derive foot contacts and run secondary motion.
#[allow(clippy::too_many_arguments)]
fn finalize_player_pose_and_palette(
    player: newengine_ecs::EntityId,
    binding: &mut PlayerAnimationRuntimeBinding,
    dt: f32,
    frame: &PlayerAnimationFrameInput,
    clip_ref: &str,
    active_state: newengine_engine_runtime::gameplay::PlayerLocomotionAnimation,
    unarmed_attack_sequence: u64,
    equipment_stance: EquipmentPresentationStance,
    transitioned: bool,
    frame_index: u64,
) -> Option<(
    Vec<Mat4>,
    Option<newengine_model_contact_api::ModelFootPoseState>,
    PlayerAnimationFinalizeTiming,
)> {
    let finalize_started = std::time::Instant::now();
    let mut timing = PlayerAnimationFinalizeTiming::default();
    let semantic = frame.semantic;
    let look_context = semantic.look_context;
    let noclip_enabled = semantic.noclip_enabled;
    let fall_presentation_requested = frame.fall_presentation_requested;
    let rifle_aim_alpha = semantic.aim_alpha;
    let rifle_recoil_alpha = semantic.recoil_alpha;
    let rifle_recoil_yaw_radians = semantic.recoil_yaw_radians;
    let rifle_obstruction_alpha = semantic.obstruction_alpha;
    let rifle_reload_progress = semantic.reload_progress;
    let equipment_presentation_active = frame.equipment_presentation_active;
    let weapon_presentation = frame.weapon_presentation.as_ref();
    let rifle_view_forward_model = frame.rifle_view_forward_model;
    let rifle_view_rotation_model = frame.rifle_view_rotation_model;
    let first_person_eye_model = frame.first_person_eye_model;
    let first_person_active = frame.first_person_active;
    let rifle_secondary_rotation_offset_local = frame.rifle_secondary_rotation_offset_local;
    let view_body_yaw_delta = frame.view_body_yaw_delta;
    let view_pitch = frame.view_pitch;
    let model_to_world = frame.model_to_world;
    let next_foot_pose_revision = frame.next_foot_pose_revision;
    let previous_foot_pose = frame.previous_foot_pose;
    let root_velocity_local = frame.root_velocity_local;
    let root_position = frame.root_position;
    let root_rotation = frame.root_rotation;
    let phase_started = std::time::Instant::now();
    synchronize_helper_pose(
        &binding.helper_pose_copies,
        &mut binding.sampled_target_locals,
    );

    binding
        .current_locals
        .clone_from(&binding.sampled_target_locals);
    timing.pose_copy_ms = phase_started.elapsed().as_secs_f32() * 1000.0;

    // Original-content look-at contract: select the authored state range, solve the view
    // intent inside its native 2D sample cloud, then give only the uncovered residual to
    // the eye range. No procedural neck/spine weights or engine-defined head angle clamps.
    let phase_started = std::time::Instant::now();
    let look_allowed = !noclip_enabled
        && !fall_presentation_requested
        && unarmed_attack_sequence == 0
        && equipment_allows_authored_head_look(equipment_stance);
    if look_allowed {
        let look_state = resolve_authored_look_state(active_state, equipment_stance, look_context);
        let _ = binding.authored_look.apply(
            look_state,
            view_body_yaw_delta,
            view_pitch,
            &mut binding.current_locals,
        );
    }

    timing.look_ms = phase_started.elapsed().as_secs_f32() * 1000.0;

    // Pose continuity belongs to the authored/base pose, before terminal procedural contacts.
    // Blending a previously visible pose *after* weapon IK reintroduces the old arm transforms and
    // visibly detaches the palms from the weapon during Ready/Aim/locomotion transitions. Blend the
    // base pose first; then let weapon IK be the last writer for the arm chains.
    let phase_started = std::time::Instant::now();
    let continuity_key = PoseContinuityKey {
        clip_hash: animation_source_hash(clip_ref),
        turn_sequence: binding.turn_sequence,
        unarmed_attack_sequence,
        equipment_stance: equipment_stance as u8,
    };
    binding
        .pose_continuity
        .apply(continuity_key, &mut binding.current_locals, dt);
    synchronize_helper_pose(&binding.helper_pose_copies, &mut binding.current_locals);
    timing.continuity_eye_ms = phase_started.elapsed().as_secs_f32() * 1000.0;

    // Terminal contacts must consume FK from this exact authored/blended frame. Reusing a frame table
    // left by endpoint composition or a previous render frame makes a mathematically valid socket solve
    // operate on the wrong pose and is sufficient to leave the rendered rifle frozen below raised arms.
    if let Err(error) = rebuild_model_joint_frames(
        &binding.animation_runtime,
        &binding.current_locals,
        &mut binding.joint_frames_scratch,
    ) {
        newengine_ulog_api::ulog::warn!(
            "fps-character: current-frame FK before equipment constraint failed player={} clip='{}': {}",
            player.stable_u64(),
            clip_ref,
            error,
        );
        return None;
    }

    let phase_started = std::time::Instant::now();
    let selected_equipment_pose_set = select_equipment_pose_set(
        &binding.equipment_default_pose_set,
        &binding.equipment_pose_sets,
        frame.equipment_pose_family.as_deref(),
    );
    let strict_rifle_family = frame.equipment_pose_family.as_deref() == Some("rifle");
    let equipment_hand_contact_pose_available =
        selected_equipment_pose_set.is_some_and(|set| match equipment_stance {
            EquipmentPresentationStance::Ready => set.ready.is_some(),
            EquipmentPresentationStance::Aim => set.has_aim(),
            EquipmentPresentationStance::Reload | EquipmentPresentationStance::None => false,
        });
    let prop_attachments = binding
        .equipment_ik
        .as_ref()
        .map(|rig| (rig.right_prop_attachment, rig.left_prop_attachment));
    let clip_owns_prop_pair = |clip: Option<&PlayerAnimationRuntimeClip>| {
        prop_attachments.is_some_and(|(right, left)| {
            right.is_some_and(|index| {
                clip.is_some_and(|clip| clip.binding.owns_skeleton_joint(index))
            }) && left.is_some_and(|index| {
                clip.is_some_and(|clip| clip.binding.owns_skeleton_joint(index))
            })
        })
    };
    let authored_equipment_prop_socket_authority_present = equipment_presentation_active
        && selected_equipment_pose_set.is_some_and(|set| {
            if let Some(transition) = binding.equipment_transition {
                // Authored ready<->aim transitions may carry the prop socket themselves. If they do,
                // that sampled frame remains authoritative and terminal palm IK must stay disabled.
                return equipment_transition_clip(set, transition.kind)
                    .zip(binding.equipment_ik.as_ref())
                    .is_some_and(|(clip, rig)| {
                        equipment_clip_owns_current_weapon_arm_contract(clip, rig)
                    });
            }
            match equipment_stance {
                EquipmentPresentationStance::Ready => set
                    .ready
                    .as_ref()
                    .zip(binding.equipment_ik.as_ref())
                    .is_some_and(|(clip, rig)| {
                        equipment_clip_owns_current_weapon_arm_contract(clip, rig)
                    }),
                EquipmentPresentationStance::Aim => {
                    if rifle_aim_alpha <= 0.001 {
                        return false;
                    }
                    let body_stance = equipment_pose_body_stance(active_state, look_context);
                    let space = set.pose_space(body_stance);
                    let terminal_grip_authority = space.grip.has_prop_socket_contract()
                        && clip_owns_prop_pair(space.grip.hands.as_ref());
                    if strict_rifle_family {
                        // TLOU MM rifle bases also write joints 18..31, but they are not the terminal
                        // weapon frame. Only the final stand HANDS / crouch PART prop layer qualifies.
                        terminal_grip_authority
                    } else {
                        equipment_aim_base_owns_current_weapon_arm_contract(
                            set,
                            body_stance,
                            frame.aim_velocity_local,
                            binding.equipment_ik.as_ref(),
                        ) || terminal_grip_authority
                    }
                }
                EquipmentPresentationStance::Reload | EquipmentPresentationStance::None => false,
            }
        });
    let runtime_rifle_prop_socket_authority_present = equipment_presentation_active
        && strict_rifle_family
        && matches!(
            equipment_stance,
            EquipmentPresentationStance::Ready | EquipmentPresentationStance::Aim
        )
        && weapon_presentation
            .zip(binding.equipment_ik.as_ref())
            .is_some_and(|(presentation, rig)| {
                current_bilateral_weapon_root(presentation, rig, &binding.joint_frames_scratch)
                    .is_some()
                    || binding
                        .equipment_transition_weapon_root
                        .is_some_and(|root| root.position.is_finite() && root.rotation.is_finite())
            });
    let equipment_prop_socket_authority_present = authored_equipment_prop_socket_authority_present
        || runtime_rifle_prop_socket_authority_present;

    let strict_rifle_contact_contract = strict_rifle_family
        && matches!(
            equipment_stance,
            EquipmentPresentationStance::Ready | EquipmentPresentationStance::Aim
        );
    let equipment_hand_contact_pose_present = if strict_rifle_contact_contract {
        // Rifle Ready/Aim is a bilateral authored contract. A partial clip/socket must never fall
        // through to palm/torso reconstruction because that creates a split pose where the firing
        // arm follows a manufactured root behind the body while the support arm stays authored.
        equipment_prop_socket_authority_present
    } else {
        equipment_hand_contact_pose_available
    };
    let strict_rifle_contract_missing =
        strict_rifle_contact_contract && !equipment_prop_socket_authority_present;
    if equipment_presentation_active {
        // Support IK is valid only when both sides of the authored binding resolved. A strict rifle
        // frame that fails bilateral fusion still retains rigid RMB aim authority through the
        // firing-side prop; only the independent two-arm repair path is suppressed.
        if let (Some(presentation), Some(rig)) =
            (weapon_presentation.as_ref(), binding.equipment_ik.as_ref())
        {
            if strict_rifle_contract_missing
                && equipment_stance == EquipmentPresentationStance::Aim
                && !first_person_active
                && rifle_aim_alpha > 1.0e-4
            {
                match apply_strict_rifle_tpp_rigid_aim_fallback(
                    presentation,
                    &binding.skeleton,
                    &binding.animation_runtime,
                    &mut binding.current_locals,
                    &mut binding.joint_frames_scratch,
                    rig,
                    rifle_aim_alpha,
                    rifle_view_forward_model,
                    &binding.equipment_aim_controller,
                ) {
                    Ok(Some(root)) => binding.equipment_resolved_weapon_root = Some(root),
                    Ok(None) => {}
                    Err(error) => {
                        if binding.equipment_ik_residual_diag_cooldown <= 0.0 {
                            newengine_ulog_api::ulog::warn!(
                                "fps-character: strict rifle rigid AIM fallback failed player={}: {}",
                                player.stable_u64(),
                                error,
                            );
                            binding.equipment_ik_residual_diag_cooldown =
                                EQUIPMENT_SUPPORT_IK_RESIDUAL_DIAG_INTERVAL_SECONDS;
                        }
                    }
                }
            } else {
                match apply_equipped_weapon_support_ik_from_current_frames(
                    presentation,
                    Some(rig),
                    &binding.skeleton,
                    &binding.animation_runtime,
                    &mut binding.current_locals,
                    &mut binding.joint_frames_scratch,
                    rifle_view_forward_model,
                    rifle_view_rotation_model,
                    first_person_eye_model,
                    first_person_active,
                    rifle_aim_alpha,
                    rifle_recoil_alpha,
                    rifle_recoil_yaw_radians,
                    rifle_obstruction_alpha,
                    rifle_secondary_rotation_offset_local,
                    equipment_hand_contact_pose_present,
                    equipment_prop_socket_authority_present,
                    strict_rifle_contact_contract,
                    binding.equipment_transition_weapon_root,
                    rifle_reload_progress
                        .map(|progress| progress <= 0.08 || progress >= 0.92)
                        .unwrap_or(true),
                    rifle_reload_progress
                        .map(|progress| progress <= 0.08 || progress >= 0.92)
                        .unwrap_or(true),
                    Some(&mut binding.equipment_aim_controller),
                ) {
                    Ok(Some(result)) => {
                        binding.equipment_resolved_weapon_root = Some(result.resolved_root);
                        if (result.error_m > EQUIPMENT_SUPPORT_IK_RESIDUAL_WARN_THRESHOLD_M
                            || result.socket_angular_error_deg
                                > EQUIPMENT_SOCKET_ANGULAR_WARN_THRESHOLD_DEG)
                            && binding.equipment_ik_residual_diag_cooldown <= 0.0
                        {
                            newengine_ulog_api::ulog::warn!(
                            "fps-character: authored equipment support IK residual player={} error_m={:.5} right_error_m={:.5} left_error_m={:.5} socket_position_error_m={:.5} socket_angular_error_deg={:.4} threshold_m={:.5} angular_threshold_deg={:.3} diagnostic_interval_s={:.1}",
                            player.stable_u64(),
                            result.error_m,
                            result.right_error_m,
                            result.left_error_m,
                            result.socket_position_error_m,
                            result.socket_angular_error_deg,
                            EQUIPMENT_SUPPORT_IK_RESIDUAL_WARN_THRESHOLD_M,
                            EQUIPMENT_SOCKET_ANGULAR_WARN_THRESHOLD_DEG,
                            EQUIPMENT_SUPPORT_IK_RESIDUAL_DIAG_INTERVAL_SECONDS,
                        );
                            binding.equipment_ik_residual_diag_cooldown =
                                EQUIPMENT_SUPPORT_IK_RESIDUAL_DIAG_INTERVAL_SECONDS;
                        }
                    }
                    Ok(None) => {
                        // Never leave the rendered weapon on a stale READY transform while the current
                        // authored rifle pose has already moved the firing-side prop socket. Bilateral
                        // solving is the preferred terminal contract, but a transient/reach failure must
                        // still publish this frame's native firing socket so weapon and arms stay attached.
                        if strict_rifle_contact_contract {
                            binding.equipment_resolved_weapon_root = rig
                                .right_prop_attachment
                                .and_then(|index| binding.joint_frames_scratch.get(index).copied())
                                .and_then(|frame| {
                                    crate::weapon_grip::weapon_root_from_authored_prop_frame(
                                        presentation,
                                        frame,
                                    )
                                });
                        }
                    }
                    Err(error) => {
                        if strict_rifle_contact_contract {
                            binding.equipment_resolved_weapon_root = rig
                                .right_prop_attachment
                                .and_then(|index| binding.joint_frames_scratch.get(index).copied())
                                .and_then(|frame| {
                                    crate::weapon_grip::weapon_root_from_authored_prop_frame(
                                        presentation,
                                        frame,
                                    )
                                });
                        }
                        if binding.equipment_ik_residual_diag_cooldown <= 0.0 {
                            newengine_ulog_api::ulog::warn!(
                                "fps-character: authored equipment support IK failed player={}: {}",
                                player.stable_u64(),
                                error,
                            );
                            binding.equipment_ik_residual_diag_cooldown =
                                EQUIPMENT_SUPPORT_IK_RESIDUAL_DIAG_INTERVAL_SECONDS;
                        }
                    }
                }
            }
        }
    }
    // Stationary TPP AIM has a hard presentation invariant: the frame may not leave the weapon on
    // the pre-AIM/READY direction while head-look already consumed live view yaw/pitch. Walking can
    // rotate the actor root and accidentally hide a missing weapon writer, so enforce the idle path
    // explicitly after all authored/support-IK work. If the normal solve already aligned the sight,
    // the rigid delta is identity; otherwise chest, both arms, prop sockets and weapon turn together.
    if equipment_presentation_active
        && strict_rifle_family
        && equipment_stance == EquipmentPresentationStance::Aim
        && !first_person_active
        && frame.native_turn_allowed
        && !strict_rifle_contract_missing
        && rifle_aim_alpha > 1.0e-4
    {
        if let (Some(presentation), Some(rig)) =
            (weapon_presentation.as_ref(), binding.equipment_ik.as_ref())
        {
            match apply_strict_rifle_tpp_rigid_aim_fallback(
                presentation,
                &binding.skeleton,
                &binding.animation_runtime,
                &mut binding.current_locals,
                &mut binding.joint_frames_scratch,
                rig,
                rifle_aim_alpha,
                rifle_view_forward_model,
                &binding.equipment_aim_controller,
            ) {
                Ok(Some(root)) => binding.equipment_resolved_weapon_root = Some(root),
                Ok(None) => {}
                Err(error) => {
                    if binding.equipment_ik_residual_diag_cooldown <= 0.0 {
                        newengine_ulog_api::ulog::warn!(
                            "fps-character: stationary rifle AIM terminal enforcement failed player={}: {}",
                            player.stable_u64(),
                            error,
                        );
                        binding.equipment_ik_residual_diag_cooldown =
                            EQUIPMENT_SUPPORT_IK_RESIDUAL_DIAG_INTERVAL_SECONDS;
                    }
                }
            }
        }
    }
    timing.support_ik_ms = phase_started.elapsed().as_secs_f32() * 1000.0;

    // Weapon contact IK is a terminal writer for shoulder/elbow/wrist. The authored deformation rig has
    // parallel *_helper / finger-roll branches that share skin weights with the anatomical hand.
    // They were synchronized before IK, so leaving them there makes the final palette contain two
    // different wrist/finger frames and visibly stretches the fingers. Re-project authored helper
    // copies after all weapon contact mutations and before palette construction.
    synchronize_helper_pose(&binding.helper_pose_copies, &mut binding.current_locals);

    let phase_started = std::time::Instant::now();
    if let Err(error) = stabilize_eye_locals(
        binding.eye_contract.as_ref(),
        &binding.skeleton,
        &mut binding.current_locals,
    ) {
        newengine_ulog_api::ulog::warn!(
            "fps-character: authored eye-local stabilization failed player={} clip='{}': {}",
            player.stable_u64(),
            clip_ref,
            error
        );
        return None;
    }
    binding
        .pose_continuity
        .commit_visible_pose(&binding.current_locals);

    timing.continuity_eye_ms += phase_started.elapsed().as_secs_f32() * 1000.0;

    let phase_started = std::time::Instant::now();
    if let Err(error) = binding
        .animation_runtime
        .build_skin_palette_from_local_pose(&binding.current_locals, &mut binding.palette_scratch)
    {
        newengine_ulog_api::ulog::warn!(
            "fps-character: player skin palette update failed player={} state='{}' clip='{}': {}",
            player.stable_u64(),
            active_state.clip_hint(),
            clip_ref,
            error
        );
        return None;
    }
    if let Err(error) = apply_detached_head_follow_palette(
        binding.head_follow.as_ref(),
        &mut binding.palette_scratch,
    ) {
        newengine_ulog_api::ulog::warn!(
            "fps-character: detached face/head follow projection failed player={} clip='{}': {}",
            player.stable_u64(),
            clip_ref,
            error
        );
        return None;
    }
    if let Err(error) =
        validate_eye_palette(binding.eye_contract.as_ref(), &binding.palette_scratch)
    {
        newengine_ulog_api::ulog::warn!(
            "fps-character: authored eye palette rejected player={} clip='{}': {}",
            player.stable_u64(),
            clip_ref,
            error
        );
        return None;
    }
    if transitioned {
        debug_dump_eye_matrices(
            binding.eye_contract.as_ref(),
            &binding.bind_joint_frames,
            &binding.current_locals,
            &binding.palette_scratch,
            &format!("transition:{clip_ref}"),
        );
    }
    timing.palette_ms = phase_started.elapsed().as_secs_f32() * 1000.0;

    let phase_started = std::time::Instant::now();
    let secondary_motion_needs_joint_frames = binding.skeletal_secondary_motion.is_some();
    binding.joint_frames_scratch.clear();
    if secondary_motion_needs_joint_frames {
        binding
            .joint_frames_scratch
            .reserve(binding.skeleton.joints.len());
        for index in 0..binding.skeleton.joints.len() {
            // Secondary motion consumes arbitrary authored chain/collider joints, so it keeps the
            // complete current-frame table. Characters without it do not pay this O(joints) pass.
            let frame = binding.palette_scratch[index] * binding.bind_joint_frames[index];
            binding.joint_frames_scratch.push(frame);
        }
    }
    let foot_pose = if noclip_enabled {
        None
    } else {
        binding.foot_joints.and_then(|feet| {
            let left_bind = *binding.bind_joint_frames.get(feet.left)?;
            let right_bind = *binding.bind_joint_frames.get(feet.right)?;
            let (left, right) = if secondary_motion_needs_joint_frames {
                (
                    *binding.joint_frames_scratch.get(feet.left)?,
                    *binding.joint_frames_scratch.get(feet.right)?,
                )
            } else {
                // Foot contact needs two absolute frames, not a reconstructed table for the whole
                // skeleton. Rebuild those two directly from the deformation palette.
                (
                    *binding.palette_scratch.get(feet.left)? * left_bind,
                    *binding.palette_scratch.get(feet.right)? * right_bind,
                )
            };

            // Skeleton foot anchors normally sit at the ankle/foot-bone origin rather than
            // on the shoe sole. Calibrate that static bind-height out before contact testing.
            // X/Z remain animated joint truth; only the authored rest height becomes y=0.
            let left_bind_y = left_bind.transform_point3(Vec3::ZERO).y.clamp(-0.30, 0.40);
            let right_bind_y = right_bind.transform_point3(Vec3::ZERO).y.clamp(-0.30, 0.40);
            let left_model = left.transform_point3(Vec3::ZERO) - Vec3::Y * left_bind_y;
            let right_model = right.transform_point3(Vec3::ZERO) - Vec3::Y * right_bind_y;
            let left_world = model_to_world.transform_point3(left_model);
            let right_world = model_to_world.transform_point3(right_model);
            Some(
                newengine_model_contact_api::ModelFootPoseState::from_world_positions(
                    next_foot_pose_revision,
                    left_world,
                    right_world,
                    previous_foot_pose,
                    dt,
                ),
            )
        })
    };
    timing.joint_frames_ms = phase_started.elapsed().as_secs_f32() * 1000.0;

    let phase_started = std::time::Instant::now();
    let (skeletal_secondary_motion, joint_frames_scratch, palette_scratch) = (
        &mut binding.skeletal_secondary_motion,
        &binding.joint_frames_scratch,
        &mut binding.palette_scratch,
    );
    if let Some(secondary_motion) = skeletal_secondary_motion.as_mut() {
        if let Err(error) = secondary_motion.tick(
            dt,
            root_velocity_local,
            root_position,
            root_rotation,
            joint_frames_scratch,
            palette_scratch,
        ) {
            newengine_ulog_api::ulog::warn!(
                "fps-character: native braid secondary motion update failed player={} clip='{}': {}",
                player.stable_u64(),
                clip_ref,
                error
            );
            return None;
        }
    }
    timing.braid_ms = phase_started.elapsed().as_secs_f32() * 1000.0;

    let phase_started = std::time::Instant::now();
    // AnimationSkeletonRuntime already validates every generated matrix for finiteness on every
    // frame. The heavier affine/max-magnitude contract is a secondary runtime acceptance guard;
    // sampling it avoids a second full palette walk in the visual hot path while transitions still
    // validate immediately before a newly authored pose is presented.
    if should_run_player_palette_runtime_validation(player.stable_u64(), frame_index, transitioned)
    {
        let expected_palette_joints = binding.skeleton.joints.len();
        if let Err(error) = super::validation::validate_player_palette(
            &binding.palette_scratch,
            expected_palette_joints,
            clip_ref,
        ) {
            newengine_ulog_api::ulog::warn!(
                "fps-character: unstable player skin palette rejected player={} state='{}' clip='{}': {}",
                player.stable_u64(),
                active_state.clip_hint(),
                clip_ref,
                error
            );
            return None;
        }
    }

    timing.validation_ms = phase_started.elapsed().as_secs_f32() * 1000.0;
    let measured_ms = timing.pose_copy_ms
        + timing.look_ms
        + timing.support_ik_ms
        + timing.continuity_eye_ms
        + timing.palette_ms
        + timing.joint_frames_ms
        + timing.braid_ms
        + timing.validation_ms;
    timing.overhead_ms = (finalize_started.elapsed().as_secs_f32() * 1000.0 - measured_ms).max(0.0);

    Some((
        std::mem::take(&mut binding.palette_scratch),
        foot_pose,
        timing,
    ))
}

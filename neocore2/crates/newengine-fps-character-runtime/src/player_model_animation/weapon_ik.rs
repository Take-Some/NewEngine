include!("weapon_ik/solver.rs");

#[inline]
fn weapon_ads_alignment_alpha(aim_alpha: f32) -> f32 {
    // Directional authority starts on the very first non-zero AIM frame. The authored animation
    // can still raise/shoulder the weapon, but it must never leave the muzzle frozen in READY while
    // the head already follows the live aim intent. Smoothstep keeps the transition continuous.
    let t = if aim_alpha.is_finite() {
        aim_alpha.clamp(0.0, 1.0)
    } else {
        0.0
    };
    t * t * (3.0 - 2.0 * t)
}

#[inline]
fn staged_weapon_sight_target(
    authored_sight_forward: Vec3,
    filtered_sight_forward: Option<Vec3>,
    aim_alpha: f32,
) -> Option<Vec3> {
    let weight = weapon_ads_alignment_alpha(aim_alpha);
    if weight <= 1.0e-5 {
        return None;
    }
    let authored = authored_sight_forward.normalize_or_zero();
    let filtered = filtered_sight_forward?.normalize_or_zero();
    if !authored.is_finite()
        || !filtered.is_finite()
        || authored.length_squared() <= 1.0e-8
        || filtered.length_squared() <= 1.0e-8
    {
        return None;
    }
    let target = authored.lerp(filtered, weight).normalize_or_zero();
    (target.is_finite() && target.length_squared() > 1.0e-8).then_some(target)
}

#[inline]
fn weapon_view_rotation_for_sight_target(
    view_rotation_model: Quat,
    sight_forward_model: Vec3,
) -> Option<Quat> {
    if !view_rotation_model.is_finite() || !sight_forward_model.is_finite() {
        return None;
    }
    let view_rotation = view_rotation_model.normalize_or_identity();
    let raw_forward = (view_rotation * -Vec3::Z).normalize_or_zero();
    let target = sight_forward_model.normalize_or_zero();
    if raw_forward.length_squared() <= 1.0e-8 || target.length_squared() <= 1.0e-8 {
        return None;
    }
    Some((Quat::from_rotation_arc(raw_forward, target) * view_rotation).normalize_or_identity())
}

#[inline]
fn current_bilateral_weapon_root(
    presentation: &newengine_engine_runtime::gameplay::WeaponPresentationDefinition,
    rig: &WeaponArmIkRig,
    frames: &[Mat4],
) -> Option<crate::weapon_grip::WeaponRootTransform> {
    let (right, left) = rig.right_prop_attachment.zip(rig.left_prop_attachment)?;
    crate::weapon_grip::weapon_root_from_bilateral_authored_prop_frames(
        presentation,
        *frames.get(right)?,
        *frames.get(left)?,
    )
    .map(|pair| pair.root)
}

/// Apply third-person RMB/free-aim as an authored upper-body delta, not as a detached weapon-root
/// correction. Native rifle prop sockets live under the wrists, so rotating both clavicle-owned arm
/// chains moves the firing socket, both hands and the muzzle together while leaving neck/head
/// authority untouched. Body turn-in-place may later consume large residual yaw and naturally
/// recenters this local arm offset.
fn apply_native_rifle_upper_body_aim_delta(
    skeleton: &ModelSkeletonMetadata,
    animation_runtime: &AnimationSkeletonRuntime,
    pose: &mut [JointLocalPose],
    frames: &mut Vec<Mat4>,
    rig: &WeaponArmIkRig,
    authored_sight_forward: Vec3,
    target_sight_forward: Vec3,
) -> Result<bool, String> {
    let from = authored_sight_forward.normalize_or_zero();
    let to = target_sight_forward.normalize_or_zero();
    if !from.is_finite()
        || !to.is_finite()
        || from.length_squared() <= 1.0e-8
        || to.length_squared() <= 1.0e-8
    {
        return Ok(false);
    }

    let mut delta = Quat::from_rotation_arc(from, to).normalize_or_identity();
    if delta.w < 0.0 {
        delta = Quat::from_xyzw(-delta.x, -delta.y, -delta.z, -delta.w);
    }
    // Keep camera/free-aim inside a believable torso envelope. Larger residual yaw belongs to
    // turn-in-place/body-follow, not to independent arm stretching.
    const MAX_UPPER_BODY_AIM_RADIANS: f32 = 55.0_f32.to_radians();
    let angle = (2.0 * delta.w.clamp(-1.0, 1.0).acos()).abs();
    if angle > MAX_UPPER_BODY_AIM_RADIANS {
        delta = Quat::IDENTITY
            .slerp(delta, MAX_UPPER_BODY_AIM_RADIANS / angle)
            .normalize_or_identity();
    }
    if delta.dot(Quat::IDENTITY).abs() >= 0.999_999_9 {
        return Ok(true);
    }

    // One chest-space writer rotates head, clavicles, both arms, authored prop sockets and weapon
    // together. This preserves the bilateral grip exactly and removes the frame-to-frame feedback
    // caused by solving the two arms independently toward a moving weapon root.
    let chest_frame = *frames
        .get(rig.chest)
        .ok_or("rifle ADS chest frame missing")?;
    let current_global = chest_frame
        .to_scale_rotation_translation()
        .1
        .normalize_or_identity();
    let desired_global = (delta * current_global).normalize_or_identity();
    set_pose_joint_global_rotation(skeleton, pose, frames, rig.chest, desired_global)?;
    refresh_model_joint_frames_subtree(animation_runtime, pose, frames, rig.chest)?;
    Ok(true)
}

fn apply_native_rifle_clavicle_aim_delta(
    skeleton: &ModelSkeletonMetadata,
    animation_runtime: &AnimationSkeletonRuntime,
    pose: &mut [JointLocalPose],
    frames: &mut Vec<Mat4>,
    rig: &WeaponArmIkRig,
    authored_sight_forward: Vec3,
    target_sight_forward: Vec3,
) -> Result<bool, String> {
    let (Some(right_clavicle), Some(left_clavicle)) = (rig.right_clavicle, rig.left_clavicle)
    else {
        return Ok(false);
    };
    let from = authored_sight_forward.normalize_or_zero();
    let to = target_sight_forward.normalize_or_zero();
    if !from.is_finite()
        || !to.is_finite()
        || from.length_squared() <= 1.0e-8
        || to.length_squared() <= 1.0e-8
    {
        return Ok(false);
    }
    let mut delta = Quat::from_rotation_arc(from, to).normalize_or_identity();
    // Keep a native arm-space envelope. Beyond this the authored body turn owns the remaining yaw;
    // do not ask shoulders to absorb a near-180-degree camera orbit in one frame.
    if delta.w < 0.0 {
        delta = Quat::from_xyzw(-delta.x, -delta.y, -delta.z, -delta.w);
    }
    const MAX_ARM_AIM_RADIANS: f32 = 65.0_f32.to_radians();
    let angle = (2.0 * delta.w.clamp(-1.0, 1.0).acos()).abs();
    if angle > MAX_ARM_AIM_RADIANS {
        delta = Quat::IDENTITY
            .slerp(delta, MAX_ARM_AIM_RADIANS / angle)
            .normalize_or_identity();
    }
    if delta.dot(Quat::IDENTITY).abs() >= 0.999_999_9 {
        return Ok(true);
    }

    let arm_roots = if right_clavicle == left_clavicle {
        [Some(right_clavicle), None]
    } else {
        [Some(right_clavicle), Some(left_clavicle)]
    };
    for clavicle in arm_roots.into_iter().flatten() {
        let current_global = frames
            .get(clavicle)
            .copied()
            .ok_or("rifle ADS clavicle frame missing")?
            .to_scale_rotation_translation()
            .1
            .normalize_or_identity();
        let desired_global = (delta * current_global).normalize_or_identity();
        set_pose_joint_global_rotation(skeleton, pose, frames, clavicle, desired_global)?;
        refresh_model_joint_frames_subtree(animation_runtime, pose, frames, clavicle)?;
    }
    Ok(true)
}

/// Apply procedural sight alignment to a native TLOU-style bilateral grip without ever splitting the
/// two authored prop branches. Both hand-prop attachments are solved toward one desired socket frame;
/// the rendered weapon root is then reconstructed from the fused pair. The anatomical palms remain
/// driven by their authored palm->socket relationship, so no arbitrary foregrip target participates.
#[allow(clippy::too_many_arguments)]
/// FPP camera-space presentation layer. Unlike terminal two-bone IK, this moves the two authored
/// clavicle-owned arm subtrees by one identical rigid transform, preserving the complete TLOU grip
/// internally while relocating that grip to the CP2077-style ironsight frame. This is local-player
/// presentation only; TPP keeps clavicles attached to the world/full-body pose and uses the stock pivot.
fn apply_first_person_bilateral_rigid_projection(
    presentation: &newengine_engine_runtime::gameplay::WeaponPresentationDefinition,
    skeleton: &ModelSkeletonMetadata,
    animation_runtime: &AnimationSkeletonRuntime,
    pose: &mut [JointLocalPose],
    frames: &mut Vec<Mat4>,
    rig: &WeaponArmIkRig,
    desired_root: crate::weapon_grip::WeaponRootTransform,
) -> Result<bool, String> {
    let (Some(right_socket), Some(left_socket), Some(right_clavicle), Some(left_clavicle)) = (
        rig.right_prop_attachment,
        rig.left_prop_attachment,
        rig.right_clavicle,
        rig.left_clavicle,
    ) else {
        return Ok(false);
    };
    let Some(current_pair) = crate::weapon_grip::weapon_root_from_bilateral_authored_prop_frames(
        presentation,
        *frames
            .get(right_socket)
            .ok_or("FPP right prop frame missing")?,
        *frames
            .get(left_socket)
            .ok_or("FPP left prop frame missing")?,
    ) else {
        return Ok(false);
    };
    let Some(current_socket) =
        crate::weapon_grip::authored_prop_frame_from_weapon_root(presentation, current_pair.root)
    else {
        return Ok(false);
    };
    let Some(desired_socket) =
        crate::weapon_grip::authored_prop_frame_from_weapon_root(presentation, desired_root)
    else {
        return Ok(false);
    };
    let delta = desired_socket * current_socket.inverse();
    let (delta_scale, delta_rotation, delta_translation) = delta.to_scale_rotation_translation();
    if !delta_scale.is_finite()
        || !delta_rotation.is_finite()
        || !delta_translation.is_finite()
        || delta_translation.length() > 0.55
    {
        return Ok(false);
    }
    let mut delta_rotation = delta_rotation.normalize_or_identity();
    if delta_rotation.w < 0.0 {
        delta_rotation = Quat::from_xyzw(
            -delta_rotation.x,
            -delta_rotation.y,
            -delta_rotation.z,
            -delta_rotation.w,
        );
    }
    let delta_angle = 2.0 * delta_rotation.w.clamp(-1.0, 1.0).acos();
    if !delta_angle.is_finite() || delta_angle > 100.0_f32.to_radians() {
        return Ok(false);
    }

    let arm_roots = if right_clavicle == left_clavicle {
        [Some(right_clavicle), None]
    } else {
        [Some(right_clavicle), Some(left_clavicle)]
    };
    for clavicle in arm_roots.into_iter().flatten() {
        let current = *frames
            .get(clavicle)
            .ok_or("FPP bilateral clavicle frame missing")?;
        set_pose_joint_global_transform(skeleton, pose, frames, clavicle, delta * current)?;
        refresh_model_joint_frames_subtree(animation_runtime, pose, frames, clavicle)?;
    }

    let Some(projected) = crate::weapon_grip::weapon_root_from_bilateral_authored_prop_frames(
        presentation,
        frames[right_socket],
        frames[left_socket],
    ) else {
        return Ok(false);
    };
    let position_error = projected.root.position.distance(desired_root.position);
    let angular_dot = projected
        .root
        .rotation
        .dot(desired_root.rotation)
        .abs()
        .clamp(0.0, 1.0);
    let angular_error_deg = (2.0 * angular_dot.acos()).to_degrees();
    Ok(position_error <= 0.001 && angular_error_deg <= 0.10)
}

fn apply_native_rifle_bilateral_root_constraint(
    presentation: &newengine_engine_runtime::gameplay::WeaponPresentationDefinition,
    skeleton: &ModelSkeletonMetadata,
    animation_runtime: &AnimationSkeletonRuntime,
    pose: &mut [JointLocalPose],
    frames: &mut Vec<Mat4>,
    rig: &WeaponArmIkRig,
    desired_root: crate::weapon_grip::WeaponRootTransform,
) -> Result<bool, String> {
    let (Some(right_socket), Some(left_socket)) =
        (rig.right_prop_attachment, rig.left_prop_attachment)
    else {
        return Ok(false);
    };
    let Some(desired_socket_frame) =
        crate::weapon_grip::authored_prop_frame_from_weapon_root(presentation, desired_root)
    else {
        return Ok(false);
    };

    let palm_target_from_socket = |palm: usize, socket: usize| -> Option<(Vec3, Quat)> {
        let palm_frame = frames.get(palm).copied()?;
        let socket_frame = frames.get(socket).copied()?;
        let palm_to_socket = palm_frame.inverse() * socket_frame;
        let desired_palm_frame = desired_socket_frame * palm_to_socket.inverse();
        let (scale, rotation, position) = desired_palm_frame.to_scale_rotation_translation();
        (scale.is_finite()
            && scale.x > 0.0
            && scale.y > 0.0
            && scale.z > 0.0
            && rotation.is_finite()
            && position.is_finite())
        .then_some((position, rotation.normalize_or_identity()))
    };
    let Some((right_target, right_rotation)) =
        palm_target_from_socket(rig.right_palm, right_socket)
    else {
        return Ok(false);
    };
    let Some((left_target, left_rotation)) = palm_target_from_socket(rig.left_palm, left_socket)
    else {
        return Ok(false);
    };

    // Transactional bilateral solve. FPP camera-space placement and TPP stock-pivot placement both
    // land here, so neither mode may ever accept a one-handed result or split the common prop frame.
    let saved = [
        (rig.right_shoulder, pose[rig.right_shoulder]),
        (rig.right_elbow, pose[rig.right_elbow]),
        (rig.right_wrist, pose[rig.right_wrist]),
        (rig.left_shoulder, pose[rig.left_shoulder]),
        (rig.left_elbow, pose[rig.left_elbow]),
        (rig.left_wrist, pose[rig.left_wrist]),
    ];
    let restore = |pose: &mut [JointLocalPose]| {
        for (index, local) in &saved {
            pose[*index] = *local;
        }
    };

    let right_pole = frames[rig.right_elbow].transform_point3(Vec3::ZERO);
    let left_pole = frames[rig.left_elbow].transform_point3(Vec3::ZERO);
    let right_solved = solve_arm_to_palm_contact(
        skeleton,
        animation_runtime,
        pose,
        frames,
        rig.right_shoulder,
        rig.right_elbow,
        rig.right_wrist,
        rig.right_palm,
        right_target,
        right_pole,
        right_rotation,
        "right-bilateral",
    )?;
    if !right_solved {
        restore(pose);
        rebuild_model_joint_frames(animation_runtime, pose, frames)?;
        return Ok(false);
    }
    let left_solved = solve_arm_to_palm_contact(
        skeleton,
        animation_runtime,
        pose,
        frames,
        rig.left_shoulder,
        rig.left_elbow,
        rig.left_wrist,
        rig.left_palm,
        left_target,
        left_pole,
        left_rotation,
        "left-bilateral",
    )?;
    if !left_solved {
        restore(pose);
        rebuild_model_joint_frames(animation_runtime, pose, frames)?;
        return Ok(false);
    }

    let coherent = crate::weapon_grip::weapon_root_from_bilateral_authored_prop_frames(
        presentation,
        frames[right_socket],
        frames[left_socket],
    )
    .is_some();
    if !coherent {
        restore(pose);
        rebuild_model_joint_frames(animation_runtime, pose, frames)?;
        return Ok(false);
    }
    Ok(true)
}

#[allow(clippy::too_many_arguments)]
fn apply_native_rifle_bilateral_weapon_constraint(
    presentation: &newengine_engine_runtime::gameplay::WeaponPresentationDefinition,
    skeleton: &ModelSkeletonMetadata,
    animation_runtime: &AnimationSkeletonRuntime,
    pose: &mut [JointLocalPose],
    frames: &mut Vec<Mat4>,
    rig: &WeaponArmIkRig,
    authored_root: crate::weapon_grip::WeaponRootTransform,
    target_sight_forward: Option<Vec3>,
) -> Result<bool, String> {
    let desired_root = if let Some(target) = target_sight_forward {
        let target = target.normalize_or_zero();
        if !target.is_finite() || target.length_squared() <= 1.0e-8 {
            return Ok(false);
        }
        let Some(root) = crate::weapon_grip::weapon_sight_aligned_root_around_stock_contact(
            presentation,
            authored_root,
            target,
        ) else {
            return Ok(false);
        };
        root
    } else {
        authored_root
    };
    apply_native_rifle_bilateral_root_constraint(
        presentation,
        skeleton,
        animation_runtime,
        pose,
        frames,
        rig,
        desired_root,
    )
}

/// Authored equipment animation owns the third-person weapon pose whenever it supplies a firing-hand
/// contact. A qualified prop socket is the strongest authority; otherwise the animated firing palm owns
/// the complete weapon root transform (translation + orientation). The anatomical ReadyHold contract is
/// only a compatibility fallback for equipment without an authored hand-contact pose. IK is terminal
/// contact stabilization for recoil/obstruction/secondary motion and must be an identity solve at the
/// canonical authored pose. Reload may release constraints.
#[cfg(test)]
#[allow(clippy::too_many_arguments)]
fn apply_equipped_weapon_support_ik(
    presentation: &newengine_engine_runtime::gameplay::WeaponPresentationDefinition,
    rig: Option<&WeaponArmIkRig>,
    skeleton: &ModelSkeletonMetadata,
    animation_runtime: &AnimationSkeletonRuntime,
    pose: &mut [JointLocalPose],
    frames: &mut Vec<Mat4>,
    view_forward_model: Option<Vec3>,
    view_rotation_model: Option<Quat>,
    first_person_eye_model: Option<Vec3>,
    first_person_active: bool,
    aim_alpha: f32,
    recoil_alpha: f32,
    recoil_yaw_radians: f32,
    obstruction_alpha: f32,
    secondary_rotation_offset_local: Vec3,
    authored_hand_contacts: bool,
    authored_prop_socket_authority: bool,
    strict_native_rifle_contact_contract: bool,
    authored_transition_weapon_root: Option<crate::weapon_grip::WeaponRootTransform>,
    support_right_hand: bool,
    support_left_hand: bool,
    aim_controller: Option<&mut WeaponAimControllerState>,
) -> Result<Option<WeaponIkSolveResult>, String> {
    if rig.is_none() {
        return Ok(None);
    }
    rebuild_model_joint_frames(animation_runtime, pose, frames)?;
    apply_equipped_weapon_support_ik_from_current_frames(
        presentation,
        rig,
        skeleton,
        animation_runtime,
        pose,
        frames,
        view_forward_model,
        view_rotation_model,
        first_person_eye_model,
        first_person_active,
        aim_alpha,
        recoil_alpha,
        recoil_yaw_radians,
        obstruction_alpha,
        secondary_rotation_offset_local,
        authored_hand_contacts,
        authored_prop_socket_authority,
        strict_native_rifle_contact_contract,
        authored_transition_weapon_root,
        support_right_hand,
        support_left_hand,
        aim_controller,
    )
}

#[allow(clippy::too_many_arguments)]
fn apply_equipped_weapon_support_ik_from_current_frames(
    presentation: &newengine_engine_runtime::gameplay::WeaponPresentationDefinition,
    rig: Option<&WeaponArmIkRig>,
    skeleton: &ModelSkeletonMetadata,
    animation_runtime: &AnimationSkeletonRuntime,
    pose: &mut [JointLocalPose],
    frames: &mut Vec<Mat4>,
    view_forward_model: Option<Vec3>,
    view_rotation_model: Option<Quat>,
    first_person_eye_model: Option<Vec3>,
    first_person_active: bool,
    aim_alpha: f32,
    recoil_alpha: f32,
    recoil_yaw_radians: f32,
    obstruction_alpha: f32,
    secondary_rotation_offset_local: Vec3,
    authored_hand_contacts: bool,
    authored_prop_socket_authority: bool,
    strict_native_rifle_contact_contract: bool,
    authored_transition_weapon_root: Option<crate::weapon_grip::WeaponRootTransform>,
    support_right_hand: bool,
    support_left_hand: bool,
    aim_controller: Option<&mut WeaponAimControllerState>,
) -> Result<Option<WeaponIkSolveResult>, String> {
    let Some(rig) = rig else {
        return Ok(None);
    };
    let chest = *frames
        .get(rig.chest)
        .ok_or("weapon ReadyHold chest frame is unavailable")?;
    let right_shoulder = *frames
        .get(rig.right_shoulder)
        .ok_or("weapon ReadyHold right shoulder frame is unavailable")?;
    let left_shoulder = *frames
        .get(rig.left_shoulder)
        .ok_or("weapon ReadyHold left shoulder frame is unavailable")?;

    // Original-content weapon presentation is authored through the character's prop domain.
    // `*_hand_prop_attachment` is a sibling of the anatomical palm under the wrist, with its own
    // authored basis; it is not interchangeable with `*_palm`. When a complete grip composition
    // qualifies that socket, the socket owns the weapon root in both TPP and full-body FPP.
    // Camera/palm-driven solving is only a compatibility path for content without that contract.

    // Prop-socket authority is stricter than generic authored hand contact. TLOU long guns author
    // *both* hand-prop attachments as one common weapon frame. Endpoints must still satisfy the tiny
    // authored pair tolerance. During a synthetic READY->AIM fallback, however, ordinary local-pose
    // blending can temporarily split the two branches by centimetres; a separately validated common
    // transition frame owns that interval and both arms are projected back onto it transactionally.
    let prop_authority_enabled = authored_hand_contacts && authored_prop_socket_authority;
    let bilateral_prop_indices = prop_authority_enabled
        .then(|| rig.right_prop_attachment.zip(rig.left_prop_attachment))
        .flatten();
    let authored_prop_pair = bilateral_prop_indices.and_then(|(right, left)| {
        crate::weapon_grip::weapon_root_from_bilateral_authored_prop_frames(
            presentation,
            *frames.get(right)?,
            *frames.get(left)?,
        )
    });
    let transition_prop_root = strict_native_rifle_contact_contract
        .then_some(authored_transition_weapon_root)
        .flatten()
        .filter(|root| root.position.is_finite() && root.rotation.is_finite())
        .filter(|_| authored_prop_pair.is_none());
    if strict_native_rifle_contact_contract
        && prop_authority_enabled
        && bilateral_prop_indices.is_some()
        && authored_prop_pair.is_none()
        && transition_prop_root.is_none()
    {
        return Err(
            "native rifle bilateral prop frames diverged beyond the authored weapon-frame contract"
                .to_owned(),
        );
    }
    let native_bilateral_prop_owned =
        authored_prop_pair.is_some() || transition_prop_root.is_some();
    let authored_pair_position_residual_m = authored_prop_pair
        .map(|pair| pair.position_residual_m)
        .unwrap_or(0.0);
    let authored_pair_angular_residual_deg = authored_prop_pair
        .map(|pair| pair.angular_residual_deg)
        .unwrap_or(0.0);
    let mut authored_prop_root = transition_prop_root
        .or_else(|| authored_prop_pair.map(|pair| pair.root))
        .or_else(|| {
            prop_authority_enabled
                .then_some(rig.right_prop_attachment)
                .flatten()
                .and_then(|index| frames.get(index).copied())
                .and_then(|frame| {
                    crate::weapon_grip::weapon_root_from_authored_prop_frame(presentation, frame)
                })
        });

    // Third-person Ready/Aim is character-authored. If there is no qualified prop socket, preserve
    // the complete firing-palm -> weapon transform recovered from the authored pose. Keeping only the
    // handle translation here and re-inventing orientation from the torso is destructive for compact
    // pistol grips and for any original-content pose that deliberately authors wrist roll/yaw.
    let authored_hand_root = (authored_prop_root.is_none()
        && authored_hand_contacts
        && support_right_hand
        && !first_person_active)
        .then(|| frames[rig.right_palm])
        .and_then(|frame| crate::weapon_grip::weapon_root_from_right_palm(presentation, frame));
    let support_anchor = (authored_prop_root.is_none()
        && authored_hand_root.is_none()
        && authored_hand_contacts
        && support_left_hand
        && !first_person_active)
        .then(|| frames[rig.left_palm])
        .and_then(|frame| {
            crate::weapon_grip::weapon_left_grip_anchor_from_left_palm(presentation, frame)
        });
    let filtered_aim_target = aim_controller
        .as_deref()
        .and_then(WeaponAimControllerState::sight_target);
    let ads_alignment_alpha = weapon_ads_alignment_alpha(aim_alpha);
    let first_person_hand_root = (authored_prop_root.is_none()
        && first_person_active
        && authored_hand_contacts
        && support_right_hand)
        .then(|| frames[rig.right_palm])
        .zip(view_rotation_model)
        .and_then(|(right_palm, view_rotation)| {
            crate::weapon_grip::weapon_first_person_hand_anchored_root(
                presentation,
                right_palm,
                view_rotation,
                filtered_aim_target,
                ads_alignment_alpha,
                recoil_alpha,
                recoil_yaw_radians,
            )
        });

    // The synthetic transition constraint is applied *before* procedural camera aiming. If the
    // subsequent sight solve cannot reach safely, its transactional rollback returns to this already
    // coherent transition pose rather than to the split local-pose blend.
    let mut native_transition_constraint_applied = false;
    if native_bilateral_prop_owned {
        if let Some(transition_root) = transition_prop_root {
            native_transition_constraint_applied = apply_native_rifle_bilateral_weapon_constraint(
                presentation,
                skeleton,
                animation_runtime,
                pose,
                frames,
                rig,
                transition_root,
                None,
            )?;
            if native_transition_constraint_applied {
                authored_prop_root = bilateral_prop_indices
                    .and_then(|(right, left)| {
                        crate::weapon_grip::weapon_root_from_bilateral_authored_prop_frames(
                            presentation,
                            frames[right],
                            frames[left],
                        )
                        .map(|pair| pair.root)
                    })
                    .or(Some(transition_root));
            }
        }
    }

    let native_aim_target = authored_prop_root.and_then(|root| {
        staged_weapon_sight_target(
            crate::weapon_grip::weapon_sight_forward(presentation, root),
            filtered_aim_target,
            aim_alpha,
        )
    });
    // Compact hand-owned firearms (pistols/sidearms) do not necessarily expose a bilateral prop
    // socket. They still need the exact same sight authority: rotate the weapon around the authored
    // firing-handle contact, then let terminal arm IK rotate the wrist/forearm to that same root.
    // Without this branch the AIM animation could raise the arms while the rendered pistol remained
    // frozen in its READY orientation.
    let authored_hand_aim_root = authored_hand_root.and_then(|root| {
        let target = staged_weapon_sight_target(
            crate::weapon_grip::weapon_sight_forward(presentation, root),
            filtered_aim_target,
            aim_alpha,
        )?;
        crate::weapon_grip::weapon_sight_aligned_root_around_handle(presentation, root, target, 1.0)
    });
    // FPP ironsight is camera-primary, matching REDengine's ProceduralIronsightData ownership.
    // Build a full-ADS root whose rear sight sits at the authored eye-relief point on the camera
    // center line, then blend the current authored root toward it. Both hands are subsequently solved
    // to that root as one bilateral prop constraint. TPP keeps the stock/shoulder pivot path below.
    let first_person_bilateral_ads_root = if first_person_active
        && native_bilateral_prop_owned
        && aim_controller.is_some()
        && ads_alignment_alpha > 1.0e-4
    {
        authored_prop_root
            .zip(first_person_eye_model)
            .zip(view_rotation_model)
            .and_then(|((authored, camera_position), view_rotation)| {
                let aim_view_rotation = filtered_aim_target
                    .and_then(|target| weapon_view_rotation_for_sight_target(view_rotation, target))
                    .unwrap_or(view_rotation);
                crate::weapon_grip::weapon_first_person_solve_contract_presented(
                    presentation,
                    camera_position,
                    aim_view_rotation,
                    right_shoulder,
                    left_shoulder,
                    1.0,
                    recoil_alpha * aim_alpha.clamp(0.0, 1.0),
                    recoil_yaw_radians * aim_alpha.clamp(0.0, 1.0),
                )
                .map(|ads| {
                    let weight = ads_alignment_alpha;
                    crate::weapon_grip::WeaponRootTransform {
                        position: authored.position.lerp(ads.root.position, weight),
                        rotation: authored
                            .rotation
                            .slerp(ads.root.rotation, weight)
                            .normalize_or_identity(),
                    }
                })
            })
    } else {
        None
    };
    let native_arm_aim_applied = if let Some(root) = authored_prop_root {
        if native_bilateral_prop_owned {
            if let Some(fpp_root) = first_person_bilateral_ads_root {
                apply_first_person_bilateral_rigid_projection(
                    presentation,
                    skeleton,
                    animation_runtime,
                    pose,
                    frames,
                    rig,
                    fpp_root,
                )?
            } else if let Some(target) = native_aim_target {
                if first_person_active {
                    apply_native_rifle_bilateral_weapon_constraint(
                        presentation,
                        skeleton,
                        animation_runtime,
                        pose,
                        frames,
                        rig,
                        root,
                        Some(target),
                    )?
                } else {
                    let authored_sight =
                        crate::weapon_grip::weapon_sight_forward(presentation, root);
                    apply_native_rifle_upper_body_aim_delta(
                        skeleton,
                        animation_runtime,
                        pose,
                        frames,
                        rig,
                        authored_sight,
                        target,
                    )?
                }
            } else {
                false
            }
        } else if let Some(target) = native_aim_target {
            let authored_sight = crate::weapon_grip::weapon_sight_forward(presentation, root);
            apply_native_rifle_clavicle_aim_delta(
                skeleton,
                animation_runtime,
                pose,
                frames,
                rig,
                authored_sight,
                target,
            )?
        } else {
            false
        }
    } else {
        false
    };
    if native_arm_aim_applied {
        authored_prop_root = if native_bilateral_prop_owned {
            bilateral_prop_indices
                .and_then(|(right, left)| {
                    crate::weapon_grip::weapon_root_from_bilateral_authored_prop_frames(
                        presentation,
                        frames[right],
                        frames[left],
                    )
                    .map(|pair| pair.root)
                })
                .or(authored_prop_root)
        } else {
            rig.right_prop_attachment
                .and_then(|index| frames.get(index).copied())
                .and_then(|frame| {
                    crate::weapon_grip::weapon_root_from_authored_prop_frame(presentation, frame)
                })
                .or(authored_prop_root)
        };
    }
    let native_sight_aim_root = if native_bilateral_prop_owned
        || native_transition_constraint_applied
        || native_arm_aim_applied
    {
        // Bilateral authority never falls back to a detached root-only rotation. The weapon is always
        // reconstructed from the same frame that both authored prop branches are constrained to.
        authored_prop_root
    } else {
        authored_prop_root
            .filter(|_| !first_person_active)
            .and_then(|root| {
                let target = match aim_controller.as_ref() {
                    Some(_) => native_aim_target,
                    None => (aim_alpha > 1.0e-4).then_some(view_forward_model).flatten(),
                }?;
                crate::weapon_grip::weapon_sight_aligned_root_around_stock_contact(
                    presentation,
                    root,
                    target,
                )
            })
    };
    let root_contract = if let Some(root) = authored_prop_root {
        let root = native_sight_aim_root.unwrap_or(root);
        // Qualified original-content grip owns the prop frame directly. Do not reinterpret its
        // sibling anatomical palm as a weapon socket and do not re-derive orientation from torso.
        crate::weapon_grip::weapon_ready_solve_contract_presented(
            presentation,
            chest,
            right_shoulder,
            left_shoulder,
            None,
            aim_alpha,
            0.0,
            0.0,
        )
        .map(|mut contract| {
            contract.root = root;
            let stock_from_handle = Vec3::new(
                presentation.stock_contact_from_handle[0],
                presentation.stock_contact_from_handle[1],
                presentation.stock_contact_from_handle[2],
            );
            contract.stock_contact = crate::weapon_grip::weapon_handle_position(presentation, root)
                + crate::weapon_grip::weapon_handle_rotation(presentation, root)
                    * stock_from_handle;
            contract
        })
    } else if let Some(root) = first_person_hand_root {
        // Compatibility FPP path for rigs without an authored prop socket: preserve the firing
        // handle while camera ADS changes sight orientation.
        crate::weapon_grip::weapon_ready_solve_contract_presented(
            presentation,
            chest,
            right_shoulder,
            left_shoulder,
            None,
            aim_alpha,
            0.0,
            0.0,
        )
        .map(|mut contract| {
            contract.root = root;
            let stock_from_handle = Vec3::new(
                presentation.stock_contact_from_handle[0],
                presentation.stock_contact_from_handle[1],
                presentation.stock_contact_from_handle[2],
            );
            contract.stock_contact = crate::weapon_grip::weapon_handle_position(presentation, root)
                + crate::weapon_grip::weapon_handle_rotation(presentation, root)
                    * stock_from_handle;
            contract
        })
    } else if first_person_active {
        // Defensive compatibility fallback for rigs that explicitly disable the firing-hand support
        // chain. Normal full-body FPP never enters this path.
        first_person_eye_model
            .zip(view_rotation_model)
            .and_then(|(eye, view_rotation)| {
                crate::weapon_grip::weapon_first_person_solve_contract_presented(
                    presentation,
                    eye,
                    view_rotation,
                    right_shoulder,
                    left_shoulder,
                    aim_alpha,
                    recoil_alpha,
                    recoil_yaw_radians,
                )
            })
            .or_else(|| {
                crate::weapon_grip::weapon_ready_solve_contract_presented(
                    presentation,
                    chest,
                    right_shoulder,
                    left_shoulder,
                    view_forward_model,
                    aim_alpha,
                    recoil_alpha,
                    recoil_yaw_radians,
                )
            })
    } else if let Some(root) = authored_hand_root {
        // The original character grip owns translation through the firing handle. During AIM the
        // sight controller may rotate that grip around the same physical handle so the pistol follows
        // camera intent exactly like the authored game pose, without detaching from the hand.
        let root = authored_hand_aim_root.unwrap_or(root);
        crate::weapon_grip::weapon_ready_solve_contract_presented(
            presentation,
            chest,
            right_shoulder,
            left_shoulder,
            None,
            aim_alpha,
            0.0,
            0.0,
        )
        .map(|mut contract| {
            contract.root = root;
            let stock_from_handle = Vec3::new(
                presentation.stock_contact_from_handle[0],
                presentation.stock_contact_from_handle[1],
                presentation.stock_contact_from_handle[2],
            );
            contract.stock_contact = crate::weapon_grip::weapon_handle_position(presentation, root)
                + crate::weapon_grip::weapon_handle_rotation(presentation, root)
                    * stock_from_handle;
            contract
        })
    } else {
        crate::weapon_grip::weapon_ready_solve_contract_presented(
            presentation,
            chest,
            right_shoulder,
            left_shoulder,
            view_forward_model,
            aim_alpha,
            recoil_alpha,
            recoil_yaw_radians,
        )
    };
    let base_contract = root_contract
        .and_then(|contract| {
            if authored_prop_root.is_some() || authored_hand_root.is_some() {
                // Third-person hand/socket-owned roots are exact attachment contracts. Contact/reach
                // fitting may stabilize limbs around them, but may never rewrite the root itself.
                Some(contract)
            } else if first_person_hand_root.is_some() {
                // FPP keeps the firing handle fixed while obstruction pivots the barrel away from the
                // blocker. With both explicit anchors omitted this helper performs only that authored
                // handle-preserving pivot; it cannot pull the weapon toward torso/support estimates.
                crate::weapon_grip::weapon_ready_contract_with_contacts(
                    presentation,
                    contract,
                    None,
                    None,
                    aim_alpha,
                    obstruction_alpha,
                )
            } else {
                crate::weapon_grip::weapon_ready_contract_with_contacts(
                    presentation,
                    contract,
                    None,
                    support_anchor,
                    aim_alpha,
                    obstruction_alpha,
                )
            }
        })
        .ok_or("weapon presentation could not resolve camera/torso contact constraint")?;
    // Reach fitting is valid only while torso/camera space owns translation. Once an authored
    // firing hand supplies the handle anchor, moving the root again would break the exact contact
    // and re-introduce the hand/root feedback loop this branch is designed to avoid.
    let base_contract = if first_person_hand_root.is_some()
        || authored_prop_root.is_some()
        || authored_hand_root.is_some()
    {
        // A hand/socket-owned root must never be translated by reach fitting: that would break the
        // exact firing-palm/handle invariant. Residual contact error belongs to the bounded arm solver.
        base_contract
    } else {
        fit_weapon_contract_to_supported_arm_reach(
            presentation,
            pose,
            frames,
            rig,
            base_contract,
            support_right_hand,
            support_left_hand,
        )
    };
    let native_prop_owned = authored_prop_root.is_some();
    let mut contract = if native_prop_owned {
        // A qualified prop socket is already the final authored weapon frame. Applying generic
        // secondary rotation here would detach the weapon from that frame and force anatomical IK
        // to compensate, which is exactly the arm inversion visible with native TLOU-style rigs.
        base_contract
    } else {
        crate::weapon_grip::weapon_ready_contract_with_secondary_rotation(
            presentation,
            base_contract,
            secondary_rotation_offset_local,
        )
        .ok_or("weapon ReadyHold could not resolve secondary constraint")?
    };
    // A qualified bilateral native rifle has already solved both authored palm->prop chains toward
    // one common frame. Generic anatomical foregrip IK must not run afterward: that would demote the
    // left side back to an arbitrary weapon-space point and destroy the bilateral authority we just
    // established. Single-socket legacy content retains the bounded support-hand compatibility solve.
    let native_view_aim = native_prop_owned && native_sight_aim_root.is_some();
    let solve_right_hand = support_right_hand
        && (!native_prop_owned
            || (!native_bilateral_prop_owned && native_view_aim && !native_arm_aim_applied));
    let solve_left_hand = support_left_hand
        && (!native_prop_owned
            || (!native_bilateral_prop_owned && strict_native_rifle_contact_contract));
    let hand_owned_root = first_person_hand_root.is_some() || authored_hand_root.is_some();

    // Native TLOU prop sockets are siblings of the anatomical palm. When ADS rotates the weapon
    // toward the real sight line, derive the firing-palm target from the *current authored*
    // palm->prop relationship instead of legacy ReadyHold offsets. This preserves Abby's native
    // wrist/socket basis while allowing camera intent to move the complete weapon+arm contract.
    let native_right_palm_target = if native_view_aim {
        rig.right_prop_attachment
            .and_then(|socket_index| frames.get(socket_index).copied())
            .and_then(|socket_frame| {
                let palm_frame = frames.get(rig.right_palm).copied()?;
                let palm_to_socket = palm_frame.inverse() * socket_frame;
                let desired_handle_position =
                    crate::weapon_grip::weapon_handle_position(presentation, contract.root);
                let desired_handle_rotation =
                    crate::weapon_grip::weapon_handle_rotation(presentation, contract.root);
                let socket_to_handle = Quat::from_xyzw(
                    presentation.authored_socket_to_weapon_handle_basis[0],
                    presentation.authored_socket_to_weapon_handle_basis[1],
                    presentation.authored_socket_to_weapon_handle_basis[2],
                    presentation.authored_socket_to_weapon_handle_basis[3],
                )
                .normalize_or_identity();
                let desired_socket_rotation =
                    (desired_handle_rotation * socket_to_handle.inverse()).normalize_or_identity();
                let desired_socket_frame = Mat4::from_scale_rotation_translation(
                    Vec3::ONE,
                    desired_socket_rotation,
                    desired_handle_position,
                );
                let desired_palm_frame = desired_socket_frame * palm_to_socket.inverse();
                let (scale, rotation, position) =
                    desired_palm_frame.to_scale_rotation_translation();
                (scale.is_finite()
                    && scale.x > 0.0
                    && scale.y > 0.0
                    && scale.z > 0.0
                    && rotation.is_finite()
                    && position.is_finite())
                .then_some((position, rotation.normalize_or_identity()))
            })
    } else {
        None
    };
    let right_target = native_right_palm_target
        .map(|target| target.0)
        .unwrap_or_else(|| {
            if hand_owned_root {
                crate::weapon_grip::weapon_hand_owned_right_palm_position(
                    presentation,
                    contract.root,
                )
            } else {
                crate::weapon_grip::weapon_ready_right_palm_position(presentation, contract.root)
            }
        });
    let right_rotation = native_right_palm_target
        .map(|target| target.1)
        .unwrap_or_else(|| {
            if hand_owned_root {
                crate::weapon_grip::weapon_hand_owned_right_palm_rotation(
                    presentation,
                    contract.root,
                )
            } else {
                crate::weapon_grip::weapon_ready_right_palm_rotation(presentation, contract.root)
            }
        });
    let native_left_palm_rotation = frames[rig.left_palm]
        .to_scale_rotation_translation()
        .1
        .normalize_or_identity();
    if solve_right_hand && !right_target.is_finite() {
        return Err("weapon ReadyHold authored right-hand target is non-finite".to_owned());
    }

    let right_solved = if solve_right_hand {
        solve_arm_to_palm_contact(
            skeleton,
            animation_runtime,
            pose,
            frames,
            rig.right_shoulder,
            rig.right_elbow,
            rig.right_wrist,
            rig.right_palm,
            right_target,
            contract.right_elbow_pole,
            right_rotation,
            "right",
        )?
    } else {
        false
    };

    // Sight alignment is never allowed to detach the rendered rifle from the firing hand. If the
    // anatomical right arm cannot reach the requested camera-aligned contract without violating the
    // safe-extension gate, keep the original authored prop root for this frame. Large yaw is then
    // resolved by body turn-in-place instead of rubber-arm stretching or a floating weapon.
    let native_view_aim_committed = if native_view_aim && !native_arm_aim_applied && !right_solved {
        if let Some(authored_root) = authored_prop_root {
            contract.root = authored_root;
            let stock_from_handle = Vec3::new(
                presentation.stock_contact_from_handle[0],
                presentation.stock_contact_from_handle[1],
                presentation.stock_contact_from_handle[2],
            );
            contract.stock_contact =
                crate::weapon_grip::weapon_handle_position(presentation, authored_root)
                    + crate::weapon_grip::weapon_handle_rotation(presentation, authored_root)
                        * stock_from_handle;
        }
        false
    } else {
        true
    };

    let left_target = if native_prop_owned && strict_native_rifle_contact_contract {
        // The character prop branch already defines the moving weapon frame. Target the visible
        // support palm directly at the weapon's authored foregrip; legacy palm offsets belong only
        // to the compatibility ReadyHold solver and may come from a different animation baseline.
        crate::weapon_grip::weapon_ready_left_grip_position(presentation, contract.root)
    } else {
        crate::weapon_grip::weapon_ready_left_palm_position(presentation, contract.root)
    };
    let left_rotation = if native_prop_owned && strict_native_rifle_contact_contract {
        native_left_palm_rotation
    } else {
        crate::weapon_grip::weapon_ready_left_palm_rotation(presentation, contract.root)
    };
    if solve_left_hand && !left_target.is_finite() {
        return Err("weapon ReadyHold authored support-hand target is non-finite".to_owned());
    }
    if solve_left_hand {
        solve_arm_to_palm_contact(
            skeleton,
            animation_runtime,
            pose,
            frames,
            rig.left_shoulder,
            rig.left_elbow,
            rig.left_wrist,
            rig.left_palm,
            left_target,
            contract.left_elbow_pole,
            left_rotation,
            "left",
        )?;
    }

    // Each arm solve incrementally refreshed its affected branch, so the shared frame table is
    // already coherent here; a final full-skeleton FK pass would duplicate work every render frame.
    let socket_indices = [
        native_prop_owned
            .then_some(rig.right_prop_attachment)
            .flatten(),
        (native_prop_owned && native_bilateral_prop_owned)
            .then_some(rig.left_prop_attachment)
            .flatten(),
    ];
    let (socket_position_error, socket_angular_error) = socket_indices
        .into_iter()
        .flatten()
        .filter_map(|index| frames.get(index).copied())
        .filter_map(|frame| {
            crate::weapon_grip::weapon_handle_frame_error_from_authored_socket(
                presentation,
                contract.root,
                frame,
            )
        })
        .fold((0.0_f32, 0.0_f32), |(position, angular), error| {
            (
                position.max(error.position_m),
                angular.max(error.angular_degrees),
            )
        });
    let socket_position_error = socket_position_error.max(authored_pair_position_residual_m);
    let socket_angular_error = socket_angular_error.max(authored_pair_angular_residual_deg);
    let right_error = if solve_right_hand && native_view_aim_committed {
        (frames[rig.right_palm].transform_point3(Vec3::ZERO) - right_target).length()
    } else {
        0.0
    };
    let left_error = if solve_left_hand {
        (frames[rig.left_palm].transform_point3(Vec3::ZERO) - left_target).length()
    } else {
        0.0
    };

    if newengine_runtime_env::var_os("NORTHSTAR_DEBUG_WEAPON_CONTACT_FRAMES").is_some() {
        static CONTACT_FRAME_SAMPLES: std::sync::atomic::AtomicUsize =
            std::sync::atomic::AtomicUsize::new(0);
        let sample = CONTACT_FRAME_SAMPLES.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        if sample < 8 {
            let handle = crate::weapon_grip::weapon_handle_position(presentation, contract.root);
            let handle_rotation =
                crate::weapon_grip::weapon_handle_rotation(presentation, contract.root);
            let left_grip =
                crate::weapon_grip::weapon_ready_left_grip_position(presentation, contract.root);
            let right_palm_frame = frames[rig.right_palm];
            let left_palm_frame = frames[rig.left_palm];
            let right_palm = right_palm_frame.transform_point3(Vec3::ZERO);
            let left_palm = left_palm_frame.transform_point3(Vec3::ZERO);
            let (_, right_palm_rotation, _) = right_palm_frame.to_scale_rotation_translation();
            let (_, left_palm_rotation, _) = left_palm_frame.to_scale_rotation_translation();
            let right_prop_frame = rig
                .right_prop_attachment
                .and_then(|index| frames.get(index).copied());
            let left_prop_frame = rig
                .left_prop_attachment
                .and_then(|index| frames.get(index).copied());
            let right_prop = right_prop_frame.map(|frame| frame.transform_point3(Vec3::ZERO));
            let left_prop = left_prop_frame.map(|frame| frame.transform_point3(Vec3::ZERO));
            let right_prop_rotation =
                right_prop_frame.map(|frame| frame.to_scale_rotation_translation().1);
            let left_prop_rotation =
                left_prop_frame.map(|frame| frame.to_scale_rotation_translation().1);
            newengine_ulog_api::ulog::info!(
                "WEAPON_CONTACT_FRAMES right_handle_error_cm={:.4} left_support_error_cm={:.4} weapon_socket_position_error_cm={:.4} weapon_socket_angular_error_deg={:.4} right_palm={:?} right_palm_rotation={:?} right_prop={:?} right_prop_rotation={:?} handle={:?} handle_rotation={:?} left_palm={:?} left_palm_rotation={:?} left_prop={:?} left_prop_rotation={:?} l_grip={:?} weapon_root_position={:?} weapon_root_rotation={:?} native_rig_to_runtime_basis={:?} authored_socket_to_weapon_handle_basis={:?} handle_from_root={:?} handle_rotation_from_root={:?}",
                right_error * 100.0,
                left_error * 100.0,
                socket_position_error * 100.0,
                socket_angular_error,
                right_palm,
                right_palm_rotation,
                right_prop,
                right_prop_rotation,
                handle,
                handle_rotation,
                left_palm,
                left_palm_rotation,
                left_prop,
                left_prop_rotation,
                left_grip,
                contract.root.position,
                contract.root.rotation,
                presentation.native_rig_to_runtime_basis,
                presentation.authored_socket_to_weapon_handle_basis,
                presentation.handle_from_root,
                presentation.handle_rotation_from_root,
            );
        }
    }
    // Stock/shoulder is intentionally a soft angular constraint. It must not be promoted to a
    // hard IK failure because different authored body proportions legitimately leave a few cm of
    // stock compression/clearance. Hand and socket/handle residuals are hard invariants.
    let error = right_error.max(left_error).max(socket_position_error);
    if !error.is_finite() || !socket_angular_error.is_finite() {
        return Err("weapon ReadyHold IK produced non-finite contact error".to_owned());
    }
    Ok(Some(WeaponIkSolveResult {
        error_m: error,
        right_error_m: right_error,
        left_error_m: left_error,
        socket_position_error_m: socket_position_error,
        socket_angular_error_deg: socket_angular_error,
        resolved_root: contract.root,
    }))
}

include!("weapon_ik/helper_pose.rs");

#![forbid(unsafe_op_in_unsafe_fn)]

mod ai;
mod ai_navigation;
mod animation_events;
mod capabilities;
mod combat;
mod components;
mod content;
mod damage;
mod events;
mod execution;
mod inventory;
mod listeners;
mod physics;
mod physics_queries;
mod player;
mod schedule;
mod snapshot;
mod ui;
mod vitals;

pub use ai::{
    apply_ai_frame_output, build_ai_frame_input, collect_ai_perception_queries,
    prepare_ai_perception, resolve_ai_perception_query_hits, step_ai_decisions, AIController,
    AIPerceptionProbe, CombatIntent, CombatIntentKind, CombatTeam, PerceptionState,
    PerceptionTuning, TargetMemory,
};
pub use ai_navigation::{
    step_ai_navigation_actuation, AINavigationState, AINavigationTuning, AIPatrolRoute,
    AIPatrolState,
};
pub use animation_events::{
    drain_animation_semantic_events, emit_animation_pulse, emit_animation_state,
    publish_animation_semantic_event, retained_animation_states, AnimationSemanticEventBus,
};
pub use capabilities::{
    dispatch_gameplay_capabilities, drain_gameplay_capability_requests,
    ensure_builtin_gameplay_capabilities, request_gameplay_capability, GameplayCapabilityBus,
    GameplayCapabilityDispatchReport, GameplayCapabilityProvider, GameplayCapabilityRegistry,
    GameplayCapabilityRequest, GAMEPLAY_CAPABILITY_AUDIO_PLAY_V1,
    GAMEPLAY_CAPABILITY_AUDIO_PRELOAD_V1,
};
pub use combat::{
    drain_interaction_events, drain_weapon_events, drain_weapon_reload_animation_markers,
    queue_weapon_reload_animation_marker, BallisticShotProfile, CombatActuationState,
    HitscanWeaponTuning, Interactable, InteractionEvent, InteractionEventBus, PendingHitscan,
    PendingInteraction, PlayerInteractionTuning, PlayerWeaponState, WeaponAccuracyModifiers,
    WeaponAccuracyState, WeaponActionKind, WeaponActionRuntime, WeaponActionTimingSource,
    WeaponAttackKind, WeaponEvent, WeaponEventBus, WeaponEventKind, WeaponFireControllerState,
    WeaponObstructionState, WeaponReloadAnimationAuthority, WeaponReloadAnimationMarker,
    WeaponReloadAnimationMarkerInbox, WeaponReloadPhase,
    WEAPON_RELOAD_ANIMATION_REQUIRED_MARKER_MASK, WEAPON_RELOAD_MARKER_AMMO_COMMITTED,
    WEAPON_RELOAD_MARKER_CHAMBERED, WEAPON_RELOAD_MARKER_COMPLETE,
    WEAPON_RELOAD_MARKER_MAGAZINE_DETACHED, WEAPON_RELOAD_MARKER_MAGAZINE_INSERTED,
};
pub use components::{
    attach_scene_element_core, attach_scene_object_core, scene_entity_by_role,
    AuthoredMapPlacement, AuthoredMapPlacementCloneSource, AuthoredMapPlacementDirty,
    AuthoredMapPlacementReplicaScaleState, AuthoredMapPlacementSource, CharacterBody,
    CharacterMotionTuning, CloudShadowRenderState, CollisionShapeDesc, DisplayMode,
    DisplayVisibility, EnvironmentDomeRenderState, EnvironmentFogRenderState,
    EnvironmentPostFxState, GameRunMode,
    GameplayActor, ModelRenderComponent, PhysicsBodyDesc, PhysicsSurface, PhysicsWorldSettings,
    PlayerActor, PlayerAnimationState, PlayerAuthoredAnimationCapabilities,
    PlayerBraidSecondaryMotionRig, PlayerCameraProfile, PlayerCameraViewMode,
    PlayerCharacterPresentation, PlayerCommandFrame, PlayerController, PlayerControllerKind,
    PlayerEvent, PlayerEventBus, PlayerEventKind, PlayerEyeParentFollowRule, PlayerFallState,
    PlayerFirstPersonBodyBarrierProfile, PlayerFirstPersonCameraAnchor,
    PlayerFirstPersonPrimitiveVariant, PlayerFixedPoseHistory, PlayerGroundState,
    PlayerJointChannels, PlayerJointCopyRule, PlayerJointRotationWeight, PlayerLandingState,
    PlayerLocomotionAnimation, PlayerLocomotionState, PlayerLookContext, PlayerModelAssignment,
    PlayerModelBinding, PlayerMovementSpeeds, PlayerPaletteFollowRule, PlayerRenderPose,
    PlayerSecondaryMotionBend, PlayerSecondaryMotionCapsule, PlayerSecondaryMotionColliderMode,
    PlayerSecondaryMotionEdge, PlayerSecondaryMotionOrientedBox, PlayerSecondaryMotionParticle,
    PlayerSecondaryMotionTuning, PlayerSkeletalSecondaryMotionRig, PlayerSkinBinding,
    PlayerSkinPose, PlayerSkinSidecarDefinition, PlayerSkinVertex, PlayerStanceKind,
    PlayerStanceState, PlayerViewState, PlayerViewVisibility, PlayerViewVisibilityPolicy,
    PlayerVisualKind, PlayerVisualPart, PlayerWeaponArmIkRigDefinition, PreparedRenderMesh,
    PrimitiveGpuEvictionQueue, ResidencyProgress, SceneAnchorFollow, SceneEntityAnchor,
    SceneEntityRole, SkyCloudProfileRenderState, StaticMeshCollider, TerrainMaterialLayers,
    WorldActivationPhase, WorldActivationState, WorldAssemblyProgress, WorldClearColor,
};
pub use content::{GameplayContentProvider, GameplayContentProviderRegistry};
pub use damage::{
    mark_character_corpse, reconcile_character_injury_state, resolve_weapon_impact,
    update_character_damage_states, BallisticMaterialResponse, CharacterDamageResponseTuning,
    CharacterDeathPhase, CharacterDeathPolicy, CharacterDeathPresentation,
    CharacterDeathTransitionState, CharacterHitReactionKind, CharacterHitReactionState,
    CharacterInjuryState, DamageHitZone, DamageHitZoneMap, DamageReceiver, DamageReceiverKind,
    DamageResolution, PendingPhysicsImpulse, WeaponImpact,
};
pub use events::{
    drain_gameplay_events, emit_gameplay_event, publish_gameplay_event, GameplayEvent,
    GameplayEventBus, GAMEPLAY_EVENT_CHARACTER_CORPSE, GAMEPLAY_EVENT_CHARACTER_DAMAGED,
    GAMEPLAY_EVENT_CHARACTER_DEATH_PRESENTATION_REQUESTED, GAMEPLAY_EVENT_CHARACTER_DIED,
    GAMEPLAY_EVENT_CHARACTER_HEALED, GAMEPLAY_EVENT_CHARACTER_HIT_REACTION,
    GAMEPLAY_EVENT_CHARACTER_INJURED, GAMEPLAY_EVENT_CHARACTER_INJURY_RECOVERED,
    GAMEPLAY_EVENT_CHARACTER_STAMINA_EXHAUSTED, GAMEPLAY_EVENT_CHARACTER_STAMINA_RECOVERED,
    GAMEPLAY_EVENT_WEAPON_EMPTY, GAMEPLAY_EVENT_WEAPON_EQUIPPED, GAMEPLAY_EVENT_WEAPON_FIRED,
    GAMEPLAY_EVENT_WEAPON_HIT, GAMEPLAY_EVENT_WEAPON_IMPACT_DEBRIS_CONTACT,
    GAMEPLAY_EVENT_WEAPON_MELEE_ATTACKED, GAMEPLAY_EVENT_WEAPON_PENETRATED,
    GAMEPLAY_EVENT_WEAPON_RELOAD_COMPLETED, GAMEPLAY_EVENT_WEAPON_RELOAD_PHASE,
    GAMEPLAY_EVENT_WEAPON_RELOAD_STARTED, GAMEPLAY_EVENT_WEAPON_SHELL_CONTACT,
    GAMEPLAY_EVENT_WEAPON_SHELL_EJECTED, GAMEPLAY_EVENT_WEAPON_SHELL_ROLLING,
    GAMEPLAY_EVENT_WEAPON_UNEQUIPPED,
};
pub use execution::{
    GameplayExecutionPhase, GameplayFrame, GameplaySystemProvider, GameplaySystemProviderRegistry,
    GameplayWorld,
};
pub use inventory::{
    active_equipped_weapon_aiming, active_equipped_weapon_binding, active_equipped_weapon_can_aim,
    active_equipped_weapon_can_fire, active_equipped_weapon_can_melee,
    active_equipped_weapon_component_modifiers, active_equipped_weapon_component_overrides,
    active_equipped_weapon_component_stat_modifiers, active_equipped_weapon_muzzle,
    active_equipped_weapon_sight, apply_loadout, consume_equipped_ammo, drain_inventory_events,
    drop_item, drop_item_instance, ensure_inventory_runtime, ensure_player_inventory,
    equip_first_item, equip_item_instance, equipped_reserve_ammo, give_item,
    install_weapon_component, inventory_capacity_state, inventory_quantity, merge_inventory_stacks,
    persist_equipped_weapon_state, play_equipped_weapon_audio, play_weapon_item_audio,
    preload_weapon_audio_definition, remove_item, remove_weapon_component,
    reorder_inventory_instance, select_equipment_slot, select_highest_ranked_equipped_weapon,
    spawn_item_pickup, spawn_persistent_item_pickup, split_inventory_stack, step_world_items,
    sync_equipped_weapon_runtime, try_collect_item_pickup, unequip_slot, use_item,
    use_item_instance, AmmoDefinition, AmmoProjectileType, EquipmentSlot, EquippedWeaponBinding,
    EquippedWeaponEntity, EquippedWeaponMuzzle, EquippedWeaponSight, FirearmWeaponDefinition,
    FiringPatternDefinition, FiringPatternKind, InventoryCapacityState, InventoryEntry,
    InventoryEvent, InventoryEventBus, InventoryEventKind, InventoryLoadout,
    InventoryLoadoutCatalog, InventoryLoadoutEntry, InventoryMutation, ItemCatalog, ItemDefinition,
    ItemId, ItemInstanceId, ItemKind, ItemPickup, ItemUseEffect, MeleeWeaponTuning,
    PlayerInventory, ResolvedWeaponStats, WeaponAdsProfile, WeaponAnimationDefinition,
    WeaponAudioAction, WeaponAudioDefinition, WeaponCapabilities, WeaponCasingDefinition,
    WeaponComponentDefinition, WeaponComponentGraphDefinition, WeaponComponentInstance,
    WeaponComponentModifiers, WeaponComponentPointDefinition, WeaponEntityRuntime,
    WeaponEntitySockets, WeaponFireMode, WeaponHandlingProfile, WeaponItemDefinition,
    WeaponPresentationDefinition, WeaponRecoilProfile, WeaponRecoilStateProfile,
    WeaponReloadTimelineProfile, WeaponReloadTopology, WeaponRuntimeProfiles, WeaponSocketPose,
    WeaponSpreadDistribution, WeaponSpreadProfile, WeaponSpreadStateProfile, WeaponStatId,
    WeaponStatModifier, WeaponStatModifierOp, WeaponStatModifierStack, WeaponSwayProfile,
    WeaponType, WeaponVfxDefinition, WorldItemDefinition, WorldItemPresentation, WorldItemRuntime,
    WorldItemVisualPart, SHARED_UNARMED_WEAPON_ITEM_NAME,
};
pub use listeners::{drain_player_events, emit_player_event, sync_player_view_listeners};
pub use physics::{prewarm_service_physics_backend, sync_prelaunch_service_physics};
pub use physics::{
    PhysicsRuntimeFrameIndex, PhysicsStaticColliderSyncProgress, PhysicsStepTimingTelemetry,
    PhysicsSyncModule,
};
pub use physics_queries::{GameplayPhysicsQueryProvider, GameplayPhysicsQueryProviderRegistry};
pub use player::{
    apply_player_command_frame, apply_player_input, apply_player_stance_geometry,
    attach_active_camera_to_player, capture_player_fixed_poses, clear_player_input,
    clear_player_model_assignment, consume_player_transient_input,
    detach_active_camera_from_player, display_shadow_caster_visible_in_mode,
    display_visible_in_mode, ensure_physics_body, first_player, is_player_controller_enabled,
    player_fall_is_confirmed, player_render_model_matrix, publish_player_render_poses,
    remove_physics_body, set_player_model_assignment, spawn_default_player,
    spawn_player_controller, update_character_animation_states, update_player_animation_states,
    update_player_stance_camera,
};
pub use schedule::{
    default_sim_schedule, run_schedule, run_schedule_with_physics_mode,
    run_schedule_with_physics_mode_and_telemetry,
    run_schedule_with_physics_mode_and_telemetry_for_frame, PhysicsIntegrationMode,
    SimulationScheduleTiming,
};
pub use snapshot::{
    capture_runtime_world_snapshot, restore_runtime_world_snapshot, RuntimeEntitySnapshot,
    RuntimeWorldSnapshot,
};
pub use ui::{
    character_vitals_hud_model, gameplay_input_capture, gameplay_modal_state,
    CharacterVitalsHudModel, GameplayInputCapture, GameplayModalState, GameplayUiFrameOutput,
    GameplayUiProvider, GameplayUiProviderRegistry, GameplayUiStatePatch,
    GameplayUiSurfaceVisibility,
};
pub use vitals::{
    reconcile_character_life_state, update_character_vitals, CharacterControlState,
    CharacterExertionState, CharacterLifeState, Health, Stamina, StaminaTuning,
};

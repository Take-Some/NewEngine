pub use newengine_physics_contracts::{CollisionShapeDesc, PhysicsBodyDesc};

use newengine_bounds::{Aabb, Bounds};
use newengine_ecs::EntityId;
use newengine_input_actions_api::ActionCommandFrame;
use newengine_math::Vec3;
use newengine_transform::Transform;

mod display;
mod physics;
mod player;
mod render_environment;
mod run_mode;
mod scene;
mod world_activation;
mod world_runtime;

pub use display::{DisplayMode, DisplayVisibility};
pub use physics::{PhysicsSurface, PhysicsWorldSettings, StaticMeshCollider};
pub use player::{
    CharacterBody, CharacterMotionTuning, PlayerAnimationState,
    PlayerAuthoredAnimationCapabilities, PlayerBraidSecondaryMotionRig, PlayerCameraProfile,
    PlayerCameraViewMode, PlayerCharacterPresentation, PlayerCommandFrame, PlayerController,
    PlayerControllerKind, PlayerEvent, PlayerEventBus, PlayerEventKind, PlayerEyeParentFollowRule,
    PlayerFallState, PlayerFirstPersonBodyBarrierProfile, PlayerFirstPersonCameraAnchor,
    PlayerFirstPersonPrimitiveVariant, PlayerFixedPoseHistory, PlayerGroundState,
    PlayerJointChannels, PlayerJointCopyRule, PlayerJointRotationWeight, PlayerLandingState,
    PlayerLocomotionAnimation, PlayerLocomotionState, PlayerLookContext, PlayerModelAssignment,
    PlayerModelBinding, PlayerMovementSpeeds, PlayerPaletteFollowRule, PlayerRenderPose,
    PlayerSecondaryMotionBend, PlayerSecondaryMotionCapsule, PlayerSecondaryMotionColliderMode,
    PlayerSecondaryMotionEdge, PlayerSecondaryMotionOrientedBox, PlayerSecondaryMotionParticle,
    PlayerSecondaryMotionTuning, PlayerSkeletalSecondaryMotionRig, PlayerSkinBinding,
    PlayerSkinPose, PlayerSkinSidecarDefinition, PlayerSkinVertex, PlayerStanceKind,
    PlayerStanceState, PlayerViewState, PlayerViewVisibility, PlayerViewVisibilityPolicy,
    PlayerVisualKind, PlayerVisualPart, PlayerWeaponArmIkRigDefinition,
};
pub use render_environment::{
    CloudShadowRenderState, EnvironmentDomeRenderState, EnvironmentFogRenderState,
    EnvironmentPostFxState, SkyCloudProfileRenderState, TerrainMaterialLayers, WorldClearColor,
};
pub use run_mode::GameRunMode;
pub use scene::{
    attach_scene_element_core, attach_scene_object_core, scene_entity_by_role,
    AuthoredMapPlacement, AuthoredMapPlacementCloneSource, AuthoredMapPlacementDirty,
    AuthoredMapPlacementReplicaScaleState, AuthoredMapPlacementSource, GameplayActor, PlayerActor,
    SceneAnchorFollow, SceneEntityAnchor, SceneEntityRole,
};
pub use world_activation::{ResidencyProgress, WorldActivationPhase, WorldActivationState};
pub use world_runtime::{
    ModelRenderComponent, PreparedRenderMesh, PrimitiveGpuEvictionQueue, WorldAssemblyProgress,
};

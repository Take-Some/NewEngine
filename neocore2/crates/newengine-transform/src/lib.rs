#![forbid(unsafe_op_in_unsafe_fn)]

#[cfg(feature = "ecs")]
mod hierarchy;
#[cfg(feature = "ecs")]
mod propagate;

#[cfg(feature = "ecs")]
mod world_space;

pub use newengine_transform_api::{
    GlobalTransform, RuntimeTransformEditOverride, Transform, TransformEditRoot, WorldPose,
};

pub use newengine_transform_api::{Children, Parent, TransformDirty};

#[cfg(feature = "ecs")]
pub use hierarchy::{despawn_hierarchies, despawn_hierarchy, set_parent};

#[cfg(feature = "ecs")]
pub use propagate::{
    ensure_transform_outputs, propagate_transforms, TransformPropagationChanges,
};

#[cfg(feature = "ecs")]
pub use world_space::{
    read_entity_world_matrix_local_chain, read_entity_world_pose,
    read_entity_world_pose_local_chain, write_entity_local_from_world_pose,
    write_entity_local_from_world_pose_local_chain,
};

#[cfg(all(feature = "ecs", feature = "service"))]
pub mod service {
    use newengine_core::{ErasedService, InterfaceId, ServiceRegistry};
    use newengine_service_api::ServiceInterface;
    use newengine_transform_api::{ITRANSFORM_RUNTIME_V1, TRANSFORM_SERVICE};

    use super::{ensure_transform_outputs, propagate_transforms, set_parent};
    use newengine_ecs::{EntityId, World};

    /// VTable for transform runtime integration.
    ///
    /// This ECS-facing ABI lives in `newengine-transform`, not
    /// `newengine-transform-api`, so the API crate remains DTO-only.
    #[repr(C)]
    pub struct TransformRuntimeVTable {
        pub ensure_outputs: unsafe fn(*mut (), *mut World),
        pub propagate: unsafe fn(*mut (), *mut World),
        pub set_parent: unsafe fn(*mut (), *mut World, EntityId, Option<EntityId>),
    }

    /// Thin, typed wrapper over `(instance_ptr, vtable_ptr)`.
    #[derive(Clone, Copy)]
    pub struct TransformRuntimeApi {
        instance: *mut (),
        vtbl: *const TransformRuntimeVTable,
    }

    unsafe impl Send for TransformRuntimeApi {}
    unsafe impl Sync for TransformRuntimeApi {}

    impl TransformRuntimeApi {
        #[inline]
        pub fn ensure_outputs(&self, world: &mut World) {
            unsafe { ((*self.vtbl).ensure_outputs)(self.instance, world as *mut _) }
        }

        #[inline]
        pub fn propagate(&self, world: &mut World) {
            unsafe { ((*self.vtbl).propagate)(self.instance, world as *mut _) }
        }

        #[inline]
        pub fn set_parent(&self, world: &mut World, child: EntityId, parent: Option<EntityId>) {
            unsafe { ((*self.vtbl).set_parent)(self.instance, world as *mut _, child, parent) }
        }
    }

    impl ServiceInterface for TransformRuntimeApi {
        type VTable = TransformRuntimeVTable;
        const INTERFACE_ID: InterfaceId = ITRANSFORM_RUNTIME_V1;

        unsafe fn from_raw(instance: *mut (), vtable: *const Self::VTable) -> Self {
            Self {
                instance,
                vtbl: vtable,
            }
        }
    }

    struct TransformRuntimeService;

    impl TransformRuntimeService {
        #[inline]
        fn new() -> Self {
            Self
        }
    }

    unsafe fn drop_service(ptr: *mut ()) {
        // SAFETY: allocated as Box<TransformRuntimeService>
        unsafe { drop(Box::from_raw(ptr as *mut TransformRuntimeService)) };
    }

    unsafe fn query_iface(_instance: *mut (), iface: InterfaceId) -> *const () {
        if iface == ITRANSFORM_RUNTIME_V1 {
            &TRANSFORM_RUNTIME_VTBL as *const TransformRuntimeVTable as *const ()
        } else {
            core::ptr::null()
        }
    }

    static TRANSFORM_RUNTIME_VTBL: TransformRuntimeVTable = TransformRuntimeVTable {
        ensure_outputs: ensure_outputs_thunk,
        propagate: propagate_thunk,
        set_parent: set_parent_thunk,
    };

    unsafe fn ensure_outputs_thunk(_instance: *mut (), world: *mut World) {
        // SAFETY: caller guarantees valid world pointer
        unsafe { ensure_transform_outputs(&mut *world) };
    }

    unsafe fn propagate_thunk(_instance: *mut (), world: *mut World) {
        unsafe { propagate_transforms(&mut *world) };
    }

    unsafe fn set_parent_thunk(
        _instance: *mut (),
        world: *mut World,
        child: EntityId,
        parent: Option<EntityId>,
    ) {
        unsafe { set_parent(&mut *world, child, parent) };
    }

    /// Registers the transform runtime service into the engine service registry.
    #[inline]
    pub fn register(registry: &ServiceRegistry) {
        let svc = Box::new(TransformRuntimeService::new());
        let instance = Box::into_raw(svc) as *mut ();
        let erased = ErasedService::new(instance, drop_service, query_iface);
        registry.register(TRANSFORM_SERVICE, erased);
    }
}

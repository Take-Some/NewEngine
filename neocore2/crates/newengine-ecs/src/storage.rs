#![forbid(unsafe_op_in_unsafe_fn)]

use core::any::{Any, TypeId};

use crate::{Component, EntityId};
use newengine_math::collections::prelude::*;
use newengine_math::collections::slotmap::SecondaryMap;

/// Type-erased component storage stored inside `World`.
///
/// `Send + Sync` are required to make `World` thread-safe.
pub trait ErasedStorage: Any + Send + Sync {
    fn as_any(&self) -> &dyn Any;
    fn as_any_mut(&mut self) -> &mut dyn Any;

    fn component_type_id(&self) -> TypeId;

    fn remove_entity(&mut self, id: EntityId);
    fn has(&self, id: EntityId) -> bool;

    fn len(&self) -> usize;
    fn is_empty(&self) -> bool;
}

/// Per-component storage with conservative change tracking.
///
/// Tracking rules:
/// - `insert` marks `added` (if new) and `changed`.
/// - `get_mut` on [`World`](crate::World) does **not** implicitly mark `changed`.
///   Use `get_mut_tracked` / `query_mut_tracked` or call `mark_changed`.
/// - `remove` does not emit events by itself (use `Events<T>` or a higher-level log).
pub struct Storage<T: Component> {
    pub(crate) map: NeSecondaryMap<EntityId, T>,
    pub(crate) added_tick: NeSecondaryMap<EntityId, u64>,
    pub(crate) changed_tick: NeSecondaryMap<EntityId, u64>,
    pub(crate) max_added_tick: u64,
    pub(crate) max_changed_tick: u64,
    pub(crate) membership_revision: u64,
}

impl<T: Component> Storage<T> {
    #[inline]
    pub fn new() -> Self {
        Self {
            map: SecondaryMap::new(),
            added_tick: SecondaryMap::new(),
            changed_tick: SecondaryMap::new(),
            max_added_tick: 0,
            max_changed_tick: 0,
            membership_revision: 0,
        }
    }

    #[inline]
    pub fn len(&self) -> usize {
        self.map.len()
    }

    #[inline]
    pub fn is_empty(&self) -> bool {
        self.map.is_empty()
    }

    #[inline]
    pub fn added_tick(&self, id: EntityId) -> Option<u64> {
        self.added_tick.get(id).copied()
    }

    #[inline]
    pub fn changed_tick(&self, id: EntityId) -> Option<u64> {
        self.changed_tick.get(id).copied()
    }

    #[inline]
    pub fn mark_changed(&mut self, id: EntityId, tick: u64) {
        if self.map.contains_key(id) {
            self.changed_tick.insert(id, tick);
            self.max_changed_tick = self.max_changed_tick.max(tick);
        }
    }

    #[inline]
    pub fn mark_added(&mut self, id: EntityId, tick: u64) {
        if self.map.contains_key(id) {
            self.added_tick.insert(id, tick);
            self.changed_tick.insert(id, tick);
            self.max_added_tick = self.max_added_tick.max(tick);
            self.max_changed_tick = self.max_changed_tick.max(tick);
        }
    }

    #[inline]
    pub fn remove_all_traces(&mut self, id: EntityId) {
        if self.map.remove(id).is_some() {
            self.membership_revision = self.membership_revision.saturating_add(1).max(1);
        }
        let _ = self.added_tick.remove(id);
        let _ = self.changed_tick.remove(id);
    }
}

impl<T: Component> Default for Storage<T> {
    #[inline]
    fn default() -> Self {
        Self::new()
    }
}

impl<T: Component> ErasedStorage for Storage<T> {
    #[inline]
    fn as_any(&self) -> &dyn Any {
        self
    }

    #[inline]
    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }

    #[inline]
    fn component_type_id(&self) -> TypeId {
        TypeId::of::<T>()
    }

    #[inline]
    fn remove_entity(&mut self, id: EntityId) {
        self.remove_all_traces(id);
    }

    #[inline]
    fn has(&self, id: EntityId) -> bool {
        self.map.contains_key(id)
    }

    #[inline]
    fn len(&self) -> usize {
        self.map.len()
    }

    #[inline]
    fn is_empty(&self) -> bool {
        self.map.is_empty()
    }
}

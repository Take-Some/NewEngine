#![forbid(unsafe_op_in_unsafe_fn)]

use crate::{
    query::{Query, Query2, Query2A, Query2B, QueryMut, QueryMutTracked},
    storage::{ErasedStorage, Storage},
    Component, EntityId,
};
use core::any::{Any, TypeId};
use newengine_math::collections::prelude::*;
use newengine_math::collections::raw::hash_map::Entry;

/// A small, deterministic ECS world.
///
/// Design goals:
/// - deterministic entity identity via generational keys
/// - type-safe component storage
/// - iteration without hidden allocations (iterators are thin wrappers)
/// - thread-safe storages/resources (Send + Sync), so scene bridges can safely share it
/// - conservative change tracking (per-component added/changed ticks)
pub struct World {
    entities: NeSlotMap<EntityId, ()>,
    storages: NeHashMap<TypeId, Box<dyn ErasedStorage>>,
    resources: NeHashMap<TypeId, Box<dyn Any + Send + Sync>>,
    tick: u64,
    entities_changed_tick: u64,
}

impl Default for World {
    #[inline]
    fn default() -> Self {
        Self::new()
    }
}

impl World {
    #[inline]
    pub fn new() -> Self {
        Self {
            entities: NeSlotMap::with_key(),
            storages: NeHashMap::default(),
            resources: NeHashMap::default(),
            tick: 1,
            entities_changed_tick: 0,
        }
    }

    /// Current world tick used for change tracking.
    ///
    /// The engine/runtime should drive this deterministically (e.g. frame index or fixed tick).
    #[inline]
    pub fn tick(&self) -> u64 {
        self.tick
    }

    /// Sets the current world tick.
    ///
    /// Use monotonically increasing values.
    #[inline]
    pub fn set_tick(&mut self, tick: u64) {
        self.tick = tick.max(1);
    }

    /// Advances the tick by 1.
    #[inline]
    pub fn advance_tick(&mut self) -> u64 {
        // Intentionally saturating: tick wrap-around breaks `since_tick` semantics.
        // u64 is effectively "never" for a game runtime, but this makes the invariant explicit.
        self.tick = self.tick.saturating_add(1).max(1);
        self.tick
    }

    #[inline]
    pub fn any_changed_since<T: Component>(&self, since_tick: u64) -> bool {
        self.storage::<T>()
            .map(|s| s.max_changed_tick > since_tick)
            .unwrap_or(false)
    }

    #[inline]
    pub fn any_added_since<T: Component>(&self, since_tick: u64) -> bool {
        self.storage::<T>()
            .map(|s| s.max_added_tick > since_tick)
            .unwrap_or(false)
    }

    #[inline]
    pub fn entities_changed_since(&self, since_tick: u64) -> bool {
        self.entities_changed_tick > since_tick
    }

    #[inline]
    pub fn spawn(&mut self) -> EntityId {
        let id = self.entities.insert(());
        self.entities_changed_tick = self.tick;
        id
    }

    #[inline]
    pub fn exists(&self, id: EntityId) -> bool {
        self.entities.contains_key(id)
    }

    #[inline]
    pub fn entity_count(&self) -> usize {
        self.entities.len()
    }

    /// Number of component storage tables currently allocated.
    #[inline]
    pub fn storage_count(&self) -> usize {
        self.storages.len()
    }

    /// Monotonic revision of the entity membership for component storage `T`.
    ///
    /// Value edits do not change this revision; adding/removing `T` or despawning an
    /// entity that owns `T` does. This lets high-frequency consumers distinguish
    /// structural changes from ordinary component updates in O(1).
    #[inline]
    pub fn component_membership_revision<T: Component>(&self) -> u64 {
        self.storage::<T>()
            .map(|s| s.membership_revision)
            .unwrap_or(0)
    }

    /// Number of ECS resources currently stored in the world.
    #[inline]
    pub fn resource_count(&self) -> usize {
        self.resources.len()
    }

    /// Last tick where the entity set changed.
    #[inline]
    pub fn entities_changed_tick(&self) -> u64 {
        self.entities_changed_tick
    }

    /// Despawns an entity and removes all its components.
    #[inline]
    pub fn despawn(&mut self, id: EntityId) -> bool {
        if self.entities.remove(id).is_none() {
            return false;
        }
        for s in self.storages.values_mut() {
            s.remove_entity(id);
        }
        self.entities_changed_tick = self.tick;
        true
    }

    #[inline]
    pub fn iter_entities(&self) -> impl Iterator<Item = EntityId> + '_ {
        self.entities.keys()
    }

    // -----------------------------
    // Resources (singletons)
    // -----------------------------

    /// Inserts (or replaces) a resource.
    #[inline]
    pub fn insert_resource<T: 'static + Send + Sync>(&mut self, r: T) {
        self.resources.insert(TypeId::of::<T>(), Box::new(r));
    }

    /// Returns an immutable resource reference.
    #[inline]
    pub fn resource<T: 'static + Send + Sync>(&self) -> Option<&T> {
        self.resources
            .get(&TypeId::of::<T>())
            .and_then(|b| b.downcast_ref::<T>())
    }

    /// Returns a mutable resource reference.
    #[inline]
    pub fn resource_mut<T: 'static + Send + Sync>(&mut self) -> Option<&mut T> {
        self.resources
            .get_mut(&TypeId::of::<T>())
            .and_then(|b| b.downcast_mut::<T>())
    }

    /// Returns a mutable resource reference, inserting the resource if missing.
    ///
    /// This is intended for allocation-free "scratch" resources used by systems.
    #[inline]
    pub fn resource_mut_or_insert_with<T: 'static + Send + Sync>(
        &mut self,
        f: impl FnOnce() -> T,
    ) -> &mut T {
        let tid = TypeId::of::<T>();
        match self.resources.entry(tid) {
            Entry::Occupied(e) => e
                .into_mut()
                .downcast_mut::<T>()
                .expect("resource type mismatch"),
            Entry::Vacant(e) => e
                .insert(Box::new(f()))
                .downcast_mut::<T>()
                .expect("resource type mismatch"),
        }
    }

    /// Returns a mutable resource reference, inserting `T::default()` if missing.
    #[inline]
    pub fn resource_mut_or_insert_default<T: Default + 'static + Send + Sync>(&mut self) -> &mut T {
        self.resource_mut_or_insert_with(T::default)
    }

    /// Removes a resource.
    #[inline]
    pub fn remove_resource<T: 'static + Send + Sync>(&mut self) -> Option<T> {
        self.resources
            .remove(&TypeId::of::<T>())
            .and_then(|b| b.downcast::<T>().ok())
            .map(|b| *b)
    }

    // -----------------------------
    // Component storages
    // -----------------------------

    /// Ensures storage for component `T` exists and returns mutable access to it.
    #[inline]
    fn storage_mut<T: Component>(&mut self) -> &mut Storage<T> {
        let tid = TypeId::of::<T>();

        match self.storages.entry(tid) {
            Entry::Occupied(e) => e
                .into_mut()
                .as_any_mut()
                .downcast_mut::<Storage<T>>()
                .expect("storage type mismatch"),
            Entry::Vacant(e) => e
                .insert(Box::new(Storage::<T>::new()))
                .as_any_mut()
                .downcast_mut::<Storage<T>>()
                .expect("storage type mismatch"),
        }
    }

    /// Returns immutable storage for `T` if it exists.
    #[inline]
    fn storage<T: Component>(&self) -> Option<&Storage<T>> {
        let tid = TypeId::of::<T>();
        self.storages
            .get(&tid)
            .and_then(|b| b.as_any().downcast_ref::<Storage<T>>())
    }

    /// Returns mutable storage for `T` if it exists (does not create it).
    #[inline]
    fn storage_mut_if_exists<T: Component>(&mut self) -> Option<&mut Storage<T>> {
        let tid = TypeId::of::<T>();
        self.storages
            .get_mut(&tid)
            .and_then(|b| b.as_any_mut().downcast_mut::<Storage<T>>())
    }

    /// Ensures component storage exists (no-op if already created).
    #[inline]
    pub fn ensure_storage<T: Component>(&mut self) {
        let _ = self.storage_mut::<T>();
    }

    /// Raw immutable access to the underlying component map.
    #[inline]
    pub fn components<T: Component>(&self) -> Option<&NeSecondaryMap<EntityId, T>> {
        Some(&self.storage::<T>()?.map)
    }

    /// Raw mutable access to the underlying component map (does not create it).
    #[inline]
    pub fn components_mut<T: Component>(&mut self) -> Option<&mut NeSecondaryMap<EntityId, T>> {
        Some(&mut self.storage_mut_if_exists::<T>()?.map)
    }

    /// Inserts (or replaces) a component on an entity.
    #[inline]
    pub fn insert<T: Component>(&mut self, id: EntityId, c: T) -> bool {
        if !self.exists(id) {
            return false;
        }

        let tick = self.tick;
        let s = self.storage_mut::<T>();

        // SecondaryMap::insert returns the previous value if it existed.
        let existed = s.map.insert(id, c).is_some();

        if existed {
            s.changed_tick.insert(id, tick);
            s.max_changed_tick = s.max_changed_tick.max(tick);
        } else {
            s.added_tick.insert(id, tick);
            s.changed_tick.insert(id, tick);
            s.max_added_tick = s.max_added_tick.max(tick);
            s.max_changed_tick = s.max_changed_tick.max(tick);
            s.membership_revision = s.membership_revision.saturating_add(1).max(1);
        }

        true
    }

    /// Removes a component from an entity (does not create storage).
    #[inline]
    pub fn remove<T: Component>(&mut self, id: EntityId) -> Option<T> {
        // Capture tick before taking a mutable borrow of storage.
        let tick = self.tick;

        // Mutate component storage.
        let s = self.storage_mut_if_exists::<T>()?;

        // Remove the component and its tracking entries.
        let v = s.map.remove(id);
        let _ = s.added_tick.remove(id);
        let _ = s.changed_tick.remove(id);

        // If the component existed, update the max tick.
        if v.is_some() {
            s.max_changed_tick = s.max_changed_tick.max(tick);
            s.membership_revision = s.membership_revision.saturating_add(1).max(1);
        }

        v
    }

    #[inline]
    pub fn get<T: Component>(&self, id: EntityId) -> Option<&T> {
        self.storage::<T>()?.map.get(id)
    }

    /// Gets a mutable component reference without side effects.
    ///
    /// This does **not** mark the component as changed. If you need change tracking,
    /// either call [`World::mark_changed`] after mutation or use [`World::get_mut_tracked`].
    #[inline]
    pub fn get_mut<T: Component>(&mut self, id: EntityId) -> Option<&mut T> {
        if !self.exists(id) {
            return None;
        }

        // No implicit storage creation on read paths.
        let s = self.storage_mut_if_exists::<T>()?;
        s.map.get_mut(id)
    }

    /// Gets a mutable component reference and marks it as changed.
    ///
    /// Use this when you *know* you are going to mutate the component and want
    /// change tracking to reflect that.
    #[inline]
    pub fn get_mut_tracked<T: Component>(&mut self, id: EntityId) -> Option<&mut T> {
        if !self.exists(id) {
            return None;
        }

        let tick = self.tick;
        let s = self.storage_mut_if_exists::<T>()?;

        // Mark-changed only if the component actually exists.
        if let Some(v) = s.map.get_mut(id) {
            s.changed_tick.insert(id, tick);
            s.max_changed_tick = s.max_changed_tick.max(tick);
            return Some(v);
        }

        None
    }

    /// Marks a component as changed for the current world tick.
    ///
    /// This is useful when you mutated a component via interior mutability or
    /// via an untracked mutable reference.
    #[inline]
    pub fn mark_changed<T: Component>(&mut self, id: EntityId) {
        if !self.exists(id) {
            return;
        }

        let tick = self.tick;
        if let Some(s) = self.storage_mut_if_exists::<T>() {
            if s.map.contains_key(id) {
                s.changed_tick.insert(id, tick);
                s.max_changed_tick = s.max_changed_tick.max(tick);
            }
        }
    }

    #[inline]
    pub fn has<T: Component>(&self, id: EntityId) -> bool {
        self.get::<T>(id).is_some()
    }

    /// Returns true if the component `T` was added after `since_tick` (strictly greater).
    #[inline]
    pub fn is_added_since<T: Component>(&self, id: EntityId, since_tick: u64) -> bool {
        self.storage::<T>()
            .and_then(|s| s.added_tick.get(id).copied())
            .map(|t| t > since_tick)
            .unwrap_or(false)
    }

    /// Returns true if the component `T` was changed after `since_tick` (strictly greater).
    #[inline]
    pub fn is_changed_since<T: Component>(&self, id: EntityId, since_tick: u64) -> bool {
        self.storage::<T>()
            .and_then(|s| s.changed_tick.get(id).copied())
            .map(|t| t > since_tick)
            .unwrap_or(false)
    }

    /// Iterates entities that have component `T` and were changed after `since_tick`.
    ///
    /// Note: this iterates the component map and checks the tick map; no allocations.
    #[inline]
    pub fn query_changed<T: Component>(
        &self,
        since_tick: u64,
    ) -> impl Iterator<Item = (EntityId, &T)> + '_ {
        self.query::<T>()
            .filter(move |(id, _)| self.is_changed_since::<T>(*id, since_tick))
    }

    /// Iterates entities that have component `T` and were added after `since_tick`.
    #[inline]
    pub fn query_added<T: Component>(
        &self,
        since_tick: u64,
    ) -> impl Iterator<Item = (EntityId, &T)> + '_ {
        self.query::<T>()
            .filter(move |(id, _)| self.is_added_since::<T>(*id, since_tick))
    }

    /// Zero-allocation query over entities that have component `T`.
    #[inline]
    pub fn query<T: Component>(&self) -> Query<'_, T> {
        Query {
            iter: self.storage::<T>().map(|s| s.map.iter()),
        }
    }

    /// Zero-allocation mutable query over entities that have component `T`.
    ///
    /// Note: this iterator does **not** update change-tracking.
    /// If you mutate components through it, prefer [`World::query_mut_tracked`]
    /// (or call [`World::mark_changed`] manually).
    #[inline]
    pub fn query_mut<T: Component>(&mut self) -> QueryMut<'_, T> {
        QueryMut {
            iter: self.storage_mut_if_exists::<T>().map(|s| s.map.iter_mut()),
        }
    }

    /// Zero-allocation mutable query that marks every yielded entity as changed.
    ///
    /// Prefer this iterator for simulation systems that mutate components.
    #[inline]
    pub fn query_mut_tracked<T: Component>(&mut self) -> Option<QueryMutTracked<'_, T>> {
        let tick = self.tick;
        let s = self.storage_mut_if_exists::<T>()?;
        let Storage {
            map,
            changed_tick,
            max_changed_tick,
            ..
        } = s;
        Some(QueryMutTracked {
            iter: map.iter_mut(),
            changed_tick,
            max_changed_tick,
            tick,
        })
    }

    /// Zero-allocation join query over entities that have both `A` and `B`.
    ///
    /// Internals: iterates the smaller component map and checks the other one.
    #[inline]
    pub fn query2<A: Component, B: Component>(&self) -> Query2<'_, A, B> {
        let a = self.storage::<A>().map(|s| &s.map);
        let b = self.storage::<B>().map(|s| &s.map);

        match (a, b) {
            (Some(am), Some(bm)) => {
                if am.len() <= bm.len() {
                    Query2::A(Query2A {
                        iter: am.iter(),
                        b: bm,
                    })
                } else {
                    Query2::B(Query2B {
                        iter: bm.iter(),
                        a: am,
                    })
                }
            }
            _ => Query2::Empty,
        }
    }

    /// Returns entity ids that have both `A` and `B`.
    ///
    /// Useful for safely performing staged updates on multiple component types.
    #[inline]
    pub fn query2_ids<A: Component, B: Component>(&self) -> impl Iterator<Item = EntityId> + '_ {
        self.query2::<A, B>().map(|(id, _, _)| id)
    }
}


#[cfg(test)]
mod membership_revision_tests {
    use super::*;

    #[derive(Clone, Copy)]
    struct Marker(u32);

    #[test]
    fn membership_revision_changes_only_for_add_remove_or_despawn() {
        let mut world = World::new();
        let entity = world.spawn();
        assert_eq!(world.component_membership_revision::<Marker>(), 0);

        assert!(world.insert(entity, Marker(1)));
        let added = world.component_membership_revision::<Marker>();
        assert!(added > 0);

        assert!(world.insert(entity, Marker(2)));
        assert_eq!(world.component_membership_revision::<Marker>(), added);
        assert_eq!(world.get::<Marker>(entity).unwrap().0, 2);

        assert!(world.remove::<Marker>(entity).is_some());
        let removed = world.component_membership_revision::<Marker>();
        assert!(removed > added);

        assert!(world.insert(entity, Marker(3)));
        let readded = world.component_membership_revision::<Marker>();
        assert!(readded > removed);
        assert!(world.despawn(entity));
        assert!(world.component_membership_revision::<Marker>() > readded);
    }
}

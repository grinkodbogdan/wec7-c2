//! Capability derivation tree (CDT).
//!
//! Every capability in the system is a node in a single forest. Roots are the
//! authority the kernel holds over an object (an endpoint, a device register
//! window, an interrupt line...). Granting authority to a component means
//! *deriving* a child node; a component delegating further derives again.
//!
//! * [`Cdt::revoke`] removes everything derived from a capability, wherever
//!   it ended up. This is how the component manager takes authority back
//!   during hot-swap or after a fault.
//! * [`Cdt::delete`] removes one capability and re-parents its children.
//! * [`CapId`]s carry a generation, so a stale id can never alias a slot that
//!   has been reused for a different capability.
//!
//! The tree stores an arbitrary payload `T` (the kernel stores object
//! reference, rights, badge and owner). Removal returns the payloads so the
//! caller can undo side effects (clear the owner's slot, unmap pages, ...).

#![no_std]

extern crate alloc;

use alloc::vec::Vec;

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub struct CapId {
    index: u32,
    generation: u32,
}

impl CapId {
    pub fn index(self) -> u32 {
        self.index
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum CdtError {
    /// The id does not name a live capability (deleted, revoked or never existed).
    Stale,
}

struct Node<T> {
    value: Option<T>,
    generation: u32,
    parent: Option<u32>,
    children: Vec<u32>,
}

pub struct Cdt<T> {
    nodes: Vec<Node<T>>,
    free: Vec<u32>,
    live: usize,
}

impl<T> Default for Cdt<T> {
    fn default() -> Self {
        Self::new()
    }
}

impl<T> Cdt<T> {
    pub const fn new() -> Self {
        Cdt { nodes: Vec::new(), free: Vec::new(), live: 0 }
    }

    /// Number of live capabilities.
    pub fn len(&self) -> usize {
        self.live
    }

    pub fn is_empty(&self) -> bool {
        self.live == 0
    }

    fn alloc(&mut self, value: T, parent: Option<u32>) -> CapId {
        self.live += 1;
        if let Some(index) = self.free.pop() {
            let node = &mut self.nodes[index as usize];
            node.value = Some(value);
            node.parent = parent;
            node.children.clear();
            CapId { index, generation: node.generation }
        } else {
            let index = self.nodes.len() as u32;
            self.nodes.push(Node { value: Some(value), generation: 0, parent, children: Vec::new() });
            CapId { index, generation: 0 }
        }
    }

    fn live_index(&self, id: CapId) -> Result<usize, CdtError> {
        match self.nodes.get(id.index as usize) {
            Some(n) if n.generation == id.generation && n.value.is_some() => Ok(id.index as usize),
            _ => Err(CdtError::Stale),
        }
    }

    fn id_of(&self, index: u32) -> CapId {
        CapId { index, generation: self.nodes[index as usize].generation }
    }

    fn release(&mut self, index: usize) -> T {
        let node = &mut self.nodes[index];
        let value = node.value.take().expect("release of dead node");
        node.generation = node.generation.wrapping_add(1);
        node.parent = None;
        node.children.clear();
        self.free.push(index as u32);
        self.live -= 1;
        value
    }

    /// Create a root capability (authority held directly by the kernel).
    pub fn insert_root(&mut self, value: T) -> CapId {
        self.alloc(value, None)
    }

    /// Derive a child of `parent`. Rights reduction is the caller's policy;
    /// the tree only records provenance.
    pub fn derive(&mut self, parent: CapId, value: T) -> Result<CapId, CdtError> {
        let p = self.live_index(parent)?;
        let id = self.alloc(value, Some(p as u32));
        self.nodes[p].children.push(id.index);
        Ok(id)
    }

    pub fn contains(&self, id: CapId) -> bool {
        self.live_index(id).is_ok()
    }

    pub fn get(&self, id: CapId) -> Option<&T> {
        let i = self.live_index(id).ok()?;
        self.nodes[i].value.as_ref()
    }

    pub fn get_mut(&mut self, id: CapId) -> Option<&mut T> {
        let i = self.live_index(id).ok()?;
        self.nodes[i].value.as_mut()
    }

    pub fn parent(&self, id: CapId) -> Option<CapId> {
        let i = self.live_index(id).ok()?;
        self.nodes[i].parent.map(|p| self.id_of(p))
    }

    pub fn children(&self, id: CapId) -> Vec<CapId> {
        match self.live_index(id) {
            Ok(i) => self.nodes[i].children.iter().map(|&c| self.id_of(c)).collect(),
            Err(_) => Vec::new(),
        }
    }

    /// Remove every descendant of `id` (but not `id` itself). Returns the
    /// removed capabilities so their side effects can be undone.
    pub fn revoke(&mut self, id: CapId) -> Result<Vec<(CapId, T)>, CdtError> {
        let i = self.live_index(id)?;
        let mut stack = core::mem::take(&mut self.nodes[i].children);
        let mut removed = Vec::new();
        while let Some(c) = stack.pop() {
            let cid = self.id_of(c);
            stack.extend(core::mem::take(&mut self.nodes[c as usize].children));
            let value = self.release(c as usize);
            removed.push((cid, value));
        }
        Ok(removed)
    }

    /// Remove `id` alone. Its children are re-parented to its parent (or
    /// become roots), so delegated authority survives the delegator's delete.
    pub fn delete(&mut self, id: CapId) -> Result<T, CdtError> {
        let i = self.live_index(id)?;
        let parent = self.nodes[i].parent;
        let children = core::mem::take(&mut self.nodes[i].children);
        for &c in &children {
            self.nodes[c as usize].parent = parent;
        }
        if let Some(p) = parent {
            let siblings = &mut self.nodes[p as usize].children;
            siblings.retain(|&s| s as usize != i);
            siblings.extend(children);
        }
        Ok(self.release(i))
    }

    /// All live capabilities.
    pub fn iter(&self) -> impl Iterator<Item = (CapId, &T)> {
        self.nodes
            .iter()
            .enumerate()
            .filter_map(|(i, n)| n.value.as_ref().map(|v| (CapId { index: i as u32, generation: n.generation }, v)))
    }
}

/// A fixed-size capability space: the slots a component addresses its
/// capabilities by.
pub struct CSpace<const N: usize> {
    slots: [Option<CapId>; N],
}

impl<const N: usize> Default for CSpace<N> {
    fn default() -> Self {
        Self::new()
    }
}

impl<const N: usize> CSpace<N> {
    pub const fn new() -> Self {
        CSpace { slots: [None; N] }
    }
    pub fn get(&self, slot: usize) -> Option<CapId> {
        self.slots.get(slot).copied().flatten()
    }
    pub fn set(&mut self, slot: usize, id: Option<CapId>) {
        self.slots[slot] = id;
    }
    pub fn first_free(&self) -> Option<usize> {
        self.slots.iter().position(|s| s.is_none())
    }
    /// Clear `slot` if it still holds `id`.
    pub fn clear_if(&mut self, slot: usize, id: CapId) {
        if self.slots.get(slot).copied().flatten() == Some(id) {
            self.slots[slot] = None;
        }
    }
    pub fn occupied(&self) -> impl Iterator<Item = (usize, CapId)> + '_ {
        self.slots.iter().enumerate().filter_map(|(i, s)| s.map(|id| (i, id)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::vec;

    #[test]
    fn revoke_removes_whole_subtree_only() {
        let mut t = Cdt::new();
        let root = t.insert_root("root");
        let a = t.derive(root, "a").unwrap();
        let a1 = t.derive(a, "a1").unwrap();
        let a2 = t.derive(a1, "a2").unwrap();
        let b = t.derive(root, "b").unwrap();

        let mut removed: Vec<_> = t.revoke(a).unwrap().into_iter().map(|(_, v)| v).collect();
        removed.sort();
        assert_eq!(removed, vec!["a1", "a2"]);
        assert!(t.contains(a));
        assert!(!t.contains(a1) && !t.contains(a2));
        assert!(t.contains(b));
        assert_eq!(t.len(), 3);
    }

    #[test]
    fn stale_ids_never_alias_reused_slots() {
        let mut t = Cdt::new();
        let root = t.insert_root(1);
        let child = t.derive(root, 2).unwrap();
        t.revoke(root).unwrap();
        let reused = t.derive(root, 3).unwrap();
        assert_eq!(reused.index(), child.index());
        assert_eq!(t.get(child), None);
        assert_eq!(t.get(reused), Some(&3));
        assert_eq!(t.derive(child, 4), Err(CdtError::Stale));
        assert_eq!(t.revoke(child).unwrap_err(), CdtError::Stale);
    }

    #[test]
    fn delete_reparents_children() {
        let mut t = Cdt::new();
        let root = t.insert_root("root");
        let mid = t.derive(root, "mid").unwrap();
        let leaf = t.derive(mid, "leaf").unwrap();
        assert_eq!(t.delete(mid), Ok("mid"));
        assert_eq!(t.parent(leaf), Some(root));
        assert_eq!(t.children(root), vec![leaf]);
        // Revoking the root still reaches the re-parented leaf.
        assert_eq!(t.revoke(root).unwrap().len(), 1);
        assert_eq!(t.len(), 1);
    }

    #[test]
    fn cspace_slots() {
        let mut t = Cdt::new();
        let mut cs: CSpace<4> = CSpace::new();
        let id = t.insert_root(());
        cs.set(0, Some(id));
        assert_eq!(cs.first_free(), Some(1));
        let other = t.insert_root(());
        cs.clear_if(0, other);
        assert_eq!(cs.get(0), Some(id));
        cs.clear_if(0, id);
        assert_eq!(cs.get(0), None);
        assert_eq!(cs.get(99), None);
    }
}

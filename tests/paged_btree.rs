//! Blind contract tests for the paged B+tree (stage 2, diff 08).
//!
//! Written from the contract only (stage-2 spec §2 P3–P6, stage-1 I1/I2/I4, the page
//! layout in `btree_page.rs` and the public API brief), without reading the
//! implementation. `check()` walks the pages through `pool().fetch` holding one pin at a
//! time and is called after every operation in the model tests.

use std::collections::{BTreeMap, HashSet};
use std::io;
use std::path::Path;

use bytemuck::Zeroable;
use index_lab::index::Index;
use index_lab::index::btree_page::{
    BTREE_MAGIC, InternalPage, KIND_INTERNAL, KIND_LEAF, LeafPage, MetaPage, NO_PAGE, kind_of,
};
use index_lab::index::paged_btree::PagedBTree;
use index_lab::storage::file::FileStorage;
use index_lab::storage::pager::Pager;
use index_lab::storage::pool::{BufferPool, PoolError};
use index_lab::storage::{MemStorage, Page, PageId, Storage};
use proptest::prelude::*;

// ---------------------------------------------------------------- helpers

fn mem_pool(cap: usize) -> BufferPool<MemStorage> {
    BufferPool::new(Pager::new(MemStorage::new()).unwrap(), cap)
}

fn file_pool(path: &Path, cap: usize) -> BufferPool<FileStorage> {
    BufferPool::new(Pager::new(FileStorage::open(path).unwrap()).unwrap(), cap)
}

fn mem_tree(max_keys: usize, frames: usize) -> PagedBTree<MemStorage> {
    PagedBTree::create(mem_pool(frames), max_keys).unwrap()
}

/// Copy a page out of the pool so the pin is released before the caller fetches more.
fn read_page<S: Storage>(pool: &BufferPool<S>, id: PageId) -> Page {
    let g = pool.fetch(id).unwrap();
    *g
}

/// Deterministic pseudo-random sequence (splitmix64) for the non-proptest bulk tests.
fn splitmix(seed: &mut u64) -> u64 {
    *seed = seed.wrapping_add(0x9E37_79B9_7F4A_7C15);
    let mut z = *seed;
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

/// Structural summary returned by `check` (for tests that want exact numbers).
#[derive(Debug, Default)]
struct Shape {
    leaves: Vec<PageId>,
    internals: usize,
    keys: Vec<(u64, u64)>,
}

/// Recursive walk state.
struct Walk {
    seen: HashSet<PageId>,
    shape: Shape,
}

/// Walk the subtree at `id`, which sits at `depth` below the root.
/// Every key in the subtree must satisfy `lo <= key < hi` (None = unbounded).
#[allow(clippy::too_many_arguments)]
fn walk<S: Storage>(
    pool: &BufferPool<S>,
    id: PageId,
    depth: u64,
    height: u64,
    max_keys: usize,
    is_root: bool,
    lo: Option<u64>,
    hi: Option<u64>,
    w: &mut Walk,
) {
    assert_ne!(id, NO_PAGE, "a child pointer is NO_PAGE (page 0 = meta)");
    assert!(
        w.seen.insert(id),
        "page {id} reachable twice (shared child / cycle)"
    );
    let page = read_page(pool, id);
    // Lower bound from the split rules (I2): non-root nodes keep at least ceil(max/2)-1 keys.
    let min_keys = max_keys.div_ceil(2) - 1;
    let in_bounds = |k: u64| lo.is_none_or(|l| l <= k) && hi.is_none_or(|h| k < h);
    match kind_of(&page) {
        Some(KIND_LEAF) => {
            assert_eq!(
                depth, height,
                "leaf {id} at depth {depth}, height() = {height}"
            );
            let l: &LeafPage = bytemuck::cast_ref(&page);
            let keys = l.keys();
            assert!(
                keys.len() <= max_keys,
                "leaf {id} holds {} > max_keys {max_keys}",
                keys.len()
            );
            if !is_root {
                assert!(
                    keys.len() >= min_keys.max(1),
                    "non-root leaf {id} holds {} keys (< {})",
                    keys.len(),
                    min_keys.max(1)
                );
            }
            assert!(
                keys.windows(2).all(|p| p[0] < p[1]),
                "leaf {id} keys not strictly ascending: {keys:?}"
            );
            for &k in keys {
                assert!(
                    in_bounds(k),
                    "leaf {id} key {k} violates separator bounds [{lo:?}, {hi:?})"
                );
            }
            w.shape.leaves.push(id);
            w.shape
                .keys
                .extend(keys.iter().copied().zip(l.values().iter().copied()));
        }
        Some(KIND_INTERNAL) => {
            assert!(
                depth < height,
                "internal node {id} at depth {depth} >= height {height}"
            );
            let n: &InternalPage = bytemuck::cast_ref(&page);
            let keys = n.keys().to_vec();
            let children = n.children().to_vec();
            assert!(
                !keys.is_empty(),
                "internal node {id} has no keys (only one child)"
            );
            assert!(
                keys.len() <= max_keys,
                "internal {id} holds {} > max_keys {max_keys}",
                keys.len()
            );
            if !is_root {
                assert!(
                    keys.len() >= min_keys,
                    "non-root internal {id} holds {} keys (< {min_keys})",
                    keys.len()
                );
            }
            assert_eq!(children.len(), keys.len() + 1, "children = keys + 1");
            assert!(
                keys.windows(2).all(|p| p[0] < p[1]),
                "internal {id} keys not strictly ascending: {keys:?}"
            );
            for &k in &keys {
                assert!(
                    in_bounds(k),
                    "internal {id} separator {k} outside its own bounds [{lo:?}, {hi:?})"
                );
            }
            w.shape.internals += 1;
            for (i, &c) in children.iter().enumerate() {
                let clo = if i == 0 { lo } else { Some(keys[i - 1]) };
                let chi = if i == keys.len() { hi } else { Some(keys[i]) };
                walk(pool, c, depth + 1, height, max_keys, false, clo, chi, w);
            }
        }
        other => panic!("page {id} is not a node (kind {other:?})"),
    }
}

/// Full structural check (P4 / I2) plus meta consistency. Holds at most one pin at a time.
fn check<S: Storage>(t: &PagedBTree<S>, max_keys: usize) -> Shape {
    let pool = t.pool();
    assert_eq!(pool.pinned_frames(), 0, "pin leaked before check");
    let meta_page = read_page(pool, 0);
    let meta: &MetaPage = bytemuck::cast_ref(&meta_page);
    assert_eq!(meta.magic, BTREE_MAGIC, "meta magic");
    assert_eq!(meta.root, t.root(), "meta.root != root()");
    assert_eq!(meta.height, t.height(), "meta.height != height()");
    assert_eq!(meta.max_keys, max_keys as u64, "meta.max_keys");
    assert_ne!(t.root(), NO_PAGE, "root is page 0");

    let mut w = Walk {
        seen: HashSet::new(),
        shape: Shape::default(),
    };
    walk(
        pool,
        t.root(),
        0,
        t.height(),
        max_keys,
        true,
        None,
        None,
        &mut w,
    );
    let shape = w.shape;
    assert!(
        shape.keys.windows(2).all(|p| p[0].0 < p[1].0),
        "in-order key sequence not strictly ascending"
    );
    if t.height() == 0 {
        assert_eq!(shape.internals, 0);
        assert_eq!(shape.leaves, vec![t.root()]);
    }

    // Leaf chain: next from the leftmost leaf visits exactly the in-order leaves.
    let leaves = &shape.leaves;
    let mut chain_keys = Vec::new();
    let mut prev = NO_PAGE;
    let mut cur = leaves[0];
    let mut visited = Vec::new();
    while cur != NO_PAGE {
        assert!(
            visited.len() < leaves.len(),
            "next chain longer than the leaf set (cycle or stray leaf)"
        );
        let p = read_page(pool, cur);
        assert_eq!(
            kind_of(&p),
            Some(KIND_LEAF),
            "next chain reached non-leaf {cur}"
        );
        let l: &LeafPage = bytemuck::cast_ref(&p);
        assert_eq!(
            l.prev, prev,
            "leaf {cur}.prev != predecessor {prev} (a.next=b ⇔ b.prev=a)"
        );
        chain_keys.extend(l.keys().iter().copied().zip(l.values().iter().copied()));
        visited.push(cur);
        prev = cur;
        cur = l.next;
    }
    assert_eq!(&visited, leaves, "next chain != in-order leaves");
    assert_eq!(chain_keys, shape.keys, "next chain keys != tree keys");

    // prev chain from the last leaf visits them in reverse, descending.
    let mut rev_keys = Vec::new();
    let mut cur = *leaves.last().unwrap();
    let mut rev_visited = Vec::new();
    while cur != NO_PAGE {
        assert!(rev_visited.len() < leaves.len(), "prev chain too long");
        let p = read_page(pool, cur);
        let l: &LeafPage = bytemuck::cast_ref(&p);
        rev_keys.extend(
            l.keys()
                .iter()
                .copied()
                .zip(l.values().iter().copied())
                .rev(),
        );
        rev_visited.push(cur);
        cur = l.prev;
    }
    rev_visited.reverse();
    assert_eq!(
        &rev_visited, leaves,
        "prev chain != reversed in-order leaves"
    );
    let mut fwd = rev_keys.clone();
    fwd.reverse();
    assert_eq!(fwd, shape.keys, "prev chain keys not descending tree keys");
    // first.prev == NO_PAGE and last.next == NO_PAGE follow from the loops terminating at
    // exactly the leaf set; assert them explicitly anyway.
    let first_p = read_page(pool, leaves[0]);
    let first: &LeafPage = bytemuck::cast_ref(&first_p);
    assert_eq!(first.prev, NO_PAGE, "first leaf prev");
    let last_p = read_page(pool, *leaves.last().unwrap());
    let last: &LeafPage = bytemuck::cast_ref(&last_p);
    assert_eq!(last.next, NO_PAGE, "last leaf next");

    // Height 0 ⇔ everything fits in the root leaf (splits happen exactly at max_keys + 1,
    // and nothing ever merges).
    assert_eq!(
        t.height() == 0,
        shape.keys.len() <= max_keys,
        "height {} with {} keys, max_keys {max_keys}",
        t.height(),
        shape.keys.len()
    );
    assert_eq!(pool.pinned_frames(), 0, "check leaked a pin");
    shape
}

fn model_range(m: &BTreeMap<u64, u64>, lo: u64, hi: u64) -> Vec<(u64, u64)> {
    if lo > hi {
        return Vec::new();
    }
    m.range(lo..=hi).map(|(&k, &v)| (k, v)).collect()
}

fn model_range_rev(m: &BTreeMap<u64, u64>, lo: u64, hi: u64) -> Vec<(u64, u64)> {
    if lo > hi {
        return Vec::new();
    }
    m.range(lo..=hi).rev().map(|(&k, &v)| (k, v)).collect()
}

/// Full comparison against the model: every model key, every key's neighbours, full range.
fn assert_matches_model<S: Storage>(t: &PagedBTree<S>, m: &BTreeMap<u64, u64>) {
    for (&k, &v) in m {
        assert_eq!(t.get(k).unwrap(), Some(v), "get({k})");
        if let Some(k1) = k.checked_add(1)
            && !m.contains_key(&k1)
        {
            assert_eq!(t.get(k1).unwrap(), None, "get({k1}) absent");
        }
    }
    let all: Vec<_> = m.iter().map(|(&k, &v)| (k, v)).collect();
    assert_eq!(t.range(0, u64::MAX).unwrap(), all, "full range");
    let mut rev = all.clone();
    rev.reverse();
    assert_eq!(t.range_rev(0, u64::MAX).unwrap(), rev, "full range_rev");
    assert_eq!(t.pool().pinned_frames(), 0);
}

fn io_kind(e: PoolError) -> io::ErrorKind {
    match e {
        PoolError::Io(e) => e.kind(),
        other => panic!("expected PoolError::Io, got {other:?}"),
    }
}

/// Storage whose page 0 is `meta` and page 1 an empty leaf.
fn storage_with_meta(meta: Page) -> MemStorage {
    let mut s = MemStorage::new();
    s.write_page(0, &meta).unwrap();
    let mut leaf = Page::zeroed();
    LeafPage::init(&mut leaf);
    s.write_page(1, &leaf).unwrap();
    s
}

fn meta_page(magic: u64, max_keys: u64) -> Page {
    let mut p = Page::zeroed();
    MetaPage::init(&mut p, 1, max_keys).magic = magic;
    p
}

fn open_err(s: MemStorage) -> PoolError {
    match PagedBTree::open(BufferPool::new(Pager::new(s).unwrap(), 2)) {
        Ok(_) => panic!("open unexpectedly succeeded"),
        Err(e) => e,
    }
}

// ---------------------------------------------------------------- create / edge cases

#[test]
fn create_writes_meta_and_empty_root_leaf() {
    let t = mem_tree(4, 2);
    assert_eq!(t.height(), 0);
    assert_eq!(t.root(), 1, "root leaf is page 1 on a fresh storage");
    let shape = check(&t, 4);
    assert_eq!(shape.leaves, vec![1]);
    assert!(shape.keys.is_empty());
    let leaf_page = read_page(t.pool(), 1);
    let l: &LeafPage = bytemuck::cast_ref(&leaf_page);
    assert_eq!(
        (l.kind, l.count, l.prev, l.next),
        (KIND_LEAF, 0, NO_PAGE, NO_PAGE)
    );
}

#[test]
fn empty_tree_get_and_ranges() {
    let t = mem_tree(3, 2);
    assert_eq!(t.get(0).unwrap(), None);
    assert_eq!(t.get(u64::MAX).unwrap(), None);
    assert_eq!(t.range(0, u64::MAX).unwrap(), vec![]);
    assert_eq!(t.range_rev(0, u64::MAX).unwrap(), vec![]);
    assert_eq!(t.range(5, 5).unwrap(), vec![]);
    assert_eq!(t.pool().pinned_frames(), 0);
}

#[test]
#[should_panic]
fn create_panics_for_max_keys_1() {
    let _ = PagedBTree::create(mem_pool(2), 1);
}

#[test]
#[should_panic]
fn create_panics_for_max_keys_1022() {
    let _ = PagedBTree::create(mem_pool(2), 1022);
}

#[test]
#[should_panic]
fn create_panics_for_max_keys_0() {
    let _ = PagedBTree::create(mem_pool(2), 0);
}

#[test]
fn create_accepts_bounds_2_and_1021() {
    for mk in [2, 1021] {
        let t = mem_tree(mk, 2);
        check(&t, mk);
    }
}

#[test]
fn upsert_returns_previous_value() {
    let mut t = mem_tree(2, 2);
    assert_eq!(t.insert(7, 70).unwrap(), None);
    assert_eq!(t.insert(7, 71).unwrap(), Some(70));
    assert_eq!(t.insert(7, 72).unwrap(), Some(71));
    assert_eq!(t.get(7).unwrap(), Some(72));
    // Upsert after the tree has split (key lives in a non-root leaf, possibly == a separator).
    for k in 0..50 {
        t.insert(k * 10, k).unwrap();
    }
    for k in 0..50 {
        assert_eq!(
            t.insert(k * 10, k + 1000).unwrap(),
            Some(k),
            "upsert {}",
            k * 10
        );
    }
    let shape = check(&t, 2);
    assert_eq!(shape.keys.len(), 51, "upsert must not add keys");
    for k in 0..50 {
        assert_eq!(t.get(k * 10).unwrap(), Some(k + 1000));
    }
    assert_eq!(t.get(7).unwrap(), Some(72));
}

#[test]
fn extreme_keys_zero_and_max() {
    let mut t = mem_tree(2, 2);
    let mut m = BTreeMap::new();
    for k in [u64::MAX, 0, u64::MAX - 1, 1, u64::MAX / 2, 2, u64::MAX - 2] {
        assert_eq!(t.insert(k, !k).unwrap(), m.insert(k, !k));
        check(&t, 2);
    }
    assert_matches_model(&t, &m);
    assert_eq!(t.get(u64::MAX).unwrap(), Some(0));
    assert_eq!(t.get(0).unwrap(), Some(u64::MAX));
    assert_eq!(t.range(u64::MAX, u64::MAX).unwrap(), vec![(u64::MAX, 0)]);
    assert_eq!(t.range(0, 0).unwrap(), vec![(0, u64::MAX)]);
    assert_eq!(
        t.range_rev(u64::MAX - 1, u64::MAX).unwrap(),
        vec![(u64::MAX, 0), (u64::MAX - 1, 1)]
    );
    // Separator equal to u64::MAX must not break the "key < hi" side.
    assert_eq!(t.insert(u64::MAX, 5).unwrap(), Some(0));
}

#[test]
fn range_lo_gt_hi_is_empty_and_lo_eq_hi_is_point() {
    let mut t = mem_tree(3, 2);
    for k in 0..100 {
        t.insert(k * 2, k).unwrap();
    }
    assert_eq!(t.range(10, 9).unwrap(), vec![]);
    assert_eq!(t.range_rev(10, 9).unwrap(), vec![]);
    assert_eq!(t.range(u64::MAX, 0).unwrap(), vec![]);
    assert_eq!(t.range_rev(u64::MAX, 0).unwrap(), vec![]);
    assert_eq!(t.range(10, 10).unwrap(), vec![(10, 5)]);
    assert_eq!(t.range_rev(10, 10).unwrap(), vec![(10, 5)]);
    assert_eq!(t.range(11, 11).unwrap(), vec![]);
    assert_eq!(t.range_rev(11, 11).unwrap(), vec![]);
    // Ranges whose ends fall between keys / outside the key span.
    assert_eq!(t.range(197, 1000).unwrap(), vec![(198, 99)]);
    assert_eq!(t.range_rev(0, 1).unwrap(), vec![(0, 0)]);
    assert_eq!(t.range(199, u64::MAX).unwrap(), vec![]);
    assert_eq!(t.pool().pinned_frames(), 0);
}

// ---------------------------------------------------------------- exact split points

#[test]
fn root_leaf_splits_exactly_at_max_keys_plus_one() {
    for mk in [2usize, 3, 4, 5, 6, 17] {
        let mut t = mem_tree(mk, 2);
        for k in 0..mk as u64 {
            t.insert(k, k).unwrap();
            assert_eq!(t.height(), 0, "max_keys {mk}: split too early at key {k}");
        }
        assert_eq!(t.root(), 1, "root unchanged before first split");
        t.insert(mk as u64, 0).unwrap();
        assert_eq!(
            t.height(),
            1,
            "max_keys {mk}: root leaf must split at max_keys+1"
        );
        assert_ne!(t.root(), 1, "a new root page replaces the old root leaf");
        let shape = check(&t, mk);
        assert_eq!(shape.leaves.len(), 2, "first split yields two leaves");
        assert_eq!(shape.internals, 1);
        // Copy-up: the root separator equals the right leaf's first key.
        let root_p = read_page(t.pool(), t.root());
        let root: &InternalPage = bytemuck::cast_ref(&root_p);
        let right_p = read_page(t.pool(), shape.leaves[1]);
        let right: &LeafPage = bytemuck::cast_ref(&right_p);
        assert_eq!(
            root.keys(),
            &[right.keys()[0]],
            "leaf split copies right's first key up"
        );
        // The separator key itself is still in a leaf (copy, not move).
        assert_eq!(t.get(right.keys()[0]).unwrap(), Some(right.values()[0]));
        // Upper half moved right: left leaf keys < right leaf keys and sizes differ by <= 1.
        let left_p = read_page(t.pool(), shape.leaves[0]);
        let left: &LeafPage = bytemuck::cast_ref(&left_p);
        assert_eq!(left.keys().len() + right.keys().len(), mk + 1);
        assert!(
            left.keys().len().abs_diff(right.keys().len()) <= 1,
            "halves"
        );
    }
}

#[test]
fn full_capacity_leaf_1021_keys_then_split() {
    let mk = 1021;
    let mut t = mem_tree(mk, 2);
    let mut m = BTreeMap::new();
    let mut seed = 42;
    while m.len() < mk {
        let k = splitmix(&mut seed);
        assert_eq!(t.insert(k, k ^ 1).unwrap(), m.insert(k, k ^ 1));
    }
    assert_eq!(t.height(), 0);
    check(&t, mk);
    let mut k = splitmix(&mut seed);
    while m.contains_key(&k) {
        k = splitmix(&mut seed);
    }
    t.insert(k, 3).unwrap();
    m.insert(k, 3);
    assert_eq!(t.height(), 1);
    check(&t, mk);
    assert_matches_model(&t, &m);
}

#[test]
fn internal_split_moves_middle_key_up() {
    // max_keys 2, ascending inserts: grow until the root is internal at height 2,
    // then verify no separator appears in two internal levels (moved, not copied).
    let mk = 2;
    let mut t = mem_tree(mk, 2);
    let mut k = 0u64;
    while t.height() < 2 {
        t.insert(k, k).unwrap();
        k += 1;
        assert!(k < 10_000);
    }
    check(&t, mk);
    let mut internal_keys = Vec::new();
    let mut stack = vec![t.root()];
    while let Some(id) = stack.pop() {
        let p = read_page(t.pool(), id);
        if kind_of(&p) == Some(KIND_INTERNAL) {
            let n: &InternalPage = bytemuck::cast_ref(&p);
            internal_keys.extend_from_slice(n.keys());
            stack.extend_from_slice(n.children());
        }
    }
    let uniq: HashSet<_> = internal_keys.iter().collect();
    assert_eq!(
        uniq.len(),
        internal_keys.len(),
        "a separator appears twice among internal nodes: {internal_keys:?}"
    );
}

#[test]
fn height_is_monotone_and_meta_tracks_it() {
    let mut t = mem_tree(3, 2);
    let mut last = 0;
    let mut seed = 7;
    for _ in 0..3000 {
        let k = splitmix(&mut seed) % 5000;
        t.insert(k, k).unwrap();
        assert!(t.height() >= last, "height decreased");
        last = t.height();
        let meta_p = read_page(t.pool(), 0);
        let meta: &MetaPage = bytemuck::cast_ref(&meta_p);
        assert_eq!((meta.root, meta.height), (t.root(), t.height()));
    }
    assert!(
        t.height() >= 4,
        "3000 keys at max_keys 3 must be deep (got {last})"
    );
    check(&t, 3);
}

// ---------------------------------------------------------------- pins (P6)

#[test]
fn two_frame_pool_works_for_a_deep_tree_and_never_leaks_pins() {
    let mk = 2;
    let mut t = mem_tree(mk, 2);
    let mut m = BTreeMap::new();
    let mut seed = 99;
    for i in 0..4000u64 {
        let k = if i % 3 == 0 {
            i
        } else {
            splitmix(&mut seed) % 100_000
        };
        assert_eq!(t.insert(k, i).unwrap(), m.insert(k, i));
        assert_eq!(t.pool().pinned_frames(), 0, "insert leaked a pin");
        let _ = t.get(k).unwrap();
        assert_eq!(t.pool().pinned_frames(), 0, "get leaked a pin");
    }
    assert!(
        t.height() >= 6,
        "expected a deep tree, height {}",
        t.height()
    );
    check(&t, mk);
    assert_matches_model(&t, &m);
    let r = t.range(1000, 50_000).unwrap();
    assert_eq!(r, model_range(&m, 1000, 50_000));
    assert_eq!(t.pool().pinned_frames(), 0, "range leaked a pin");
    let r = t.range_rev(1000, 50_000).unwrap();
    assert_eq!(r, model_range_rev(&m, 1000, 50_000));
    assert_eq!(t.pool().pinned_frames(), 0, "range_rev leaked a pin");
    assert!(
        t.pool().stats().evictions > 0,
        "2-frame pool must have evicted"
    );
}

#[test]
fn pins_zero_after_every_op_with_descending_inserts() {
    // Interleave all operations with a 2-frame pool; pins must be 0 after each.
    let mut t = mem_tree(4, 2);
    for k in (0..500u64).rev() {
        t.insert(k * 3, k).unwrap();
        assert_eq!(t.pool().pinned_frames(), 0);
        t.range(k, k + 30).unwrap();
        assert_eq!(t.pool().pinned_frames(), 0);
        t.range_rev(k, k + 30).unwrap();
        assert_eq!(t.pool().pinned_frames(), 0);
        t.get(k).unwrap();
        assert_eq!(t.pool().pinned_frames(), 0);
        let _ = t.height();
        let _ = t.root();
    }
    check(&t, 4);
}

// ---------------------------------------------------------------- Index trait

#[test]
fn index_trait_object_matches_model() {
    let mut t = mem_tree(3, 3);
    let mut m = BTreeMap::new();
    {
        let idx: &mut dyn Index = &mut t;
        let mut seed = 5;
        for i in 0..500 {
            let k = splitmix(&mut seed) % 300;
            assert_eq!(idx.insert(k, i), m.insert(k, i));
        }
        for k in 0..310 {
            assert_eq!(idx.get(k), m.get(&k).copied());
        }
        assert_eq!(idx.range(50, 150), model_range(&m, 50, 150));
        assert_eq!(idx.range_rev(50, 150), model_range_rev(&m, 50, 150));
        assert_eq!(idx.range(150, 50), vec![]);
    }
    check(&t, 3);
}

// ---------------------------------------------------------------- persistence (P5)

#[test]
fn persistence_reopen_preserves_everything_and_keeps_working() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("bt.db");
    let mk = 4;
    let mut m = BTreeMap::new();
    let mut seed = 2026;
    let (root, height) = {
        let mut t = PagedBTree::create(file_pool(&path, 3), mk).unwrap();
        for i in 0..3000u64 {
            let k = splitmix(&mut seed) % 20_000;
            assert_eq!(t.insert(k, i).unwrap(), m.insert(k, i));
        }
        check(&t, mk);
        t.pool().flush().unwrap();
        (t.root(), t.height())
    }; // dropped: file closed

    let mut t = PagedBTree::open(file_pool(&path, 2)).unwrap();
    assert_eq!(
        (t.root(), t.height()),
        (root, height),
        "root/height after reopen"
    );
    check(&t, mk);
    assert_matches_model(&t, &m);
    assert_eq!(t.range(5000, 9000).unwrap(), model_range(&m, 5000, 9000));
    assert_eq!(
        t.range_rev(5000, 9000).unwrap(),
        model_range_rev(&m, 5000, 9000)
    );

    // Keep inserting after reopen: max_keys must come from meta, new pages must not
    // overwrite old ones.
    for i in 0..2000u64 {
        let k = splitmix(&mut seed) % 40_000;
        assert_eq!(
            t.insert(k, i + 1_000_000).unwrap(),
            m.insert(k, i + 1_000_000)
        );
    }
    check(&t, mk);
    assert_matches_model(&t, &m);

    // And a second round trip.
    t.pool().flush().unwrap();
    let (root, height) = (t.root(), t.height());
    drop(t);
    let t = PagedBTree::open(file_pool(&path, 4)).unwrap();
    assert_eq!((t.root(), t.height()), (root, height));
    check(&t, mk);
    assert_matches_model(&t, &m);
}

#[test]
fn persistence_of_empty_and_single_leaf_trees() {
    let dir = tempfile::tempdir().unwrap();
    for (n, name) in [(0u64, "empty.db"), (3, "one.db")] {
        let path = dir.path().join(name);
        {
            let mut t = PagedBTree::create(file_pool(&path, 2), 5).unwrap();
            for k in 0..n {
                t.insert(k, k + 1).unwrap();
            }
            t.pool().flush().unwrap();
        }
        let t = PagedBTree::open(file_pool(&path, 2)).unwrap();
        assert_eq!((t.root(), t.height()), (1, 0));
        let shape = check(&t, 5);
        assert_eq!(shape.keys, (0..n).map(|k| (k, k + 1)).collect::<Vec<_>>());
    }
}

// ---------------------------------------------------------------- open() rejects

#[test]
fn open_rejects_zero_page_0() {
    let s = storage_with_meta(Page::zeroed());
    assert_eq!(io_kind(open_err(s)), io::ErrorKind::InvalidData);
}

#[test]
fn open_rejects_wrong_magic() {
    let s = storage_with_meta(meta_page(u64::from_le_bytes(*b"IDXHASH_"), 4));
    assert_eq!(io_kind(open_err(s)), io::ErrorKind::InvalidData);
    let s = storage_with_meta(meta_page(BTREE_MAGIC ^ 1, 4));
    assert_eq!(io_kind(open_err(s)), io::ErrorKind::InvalidData);
}

#[test]
fn open_rejects_max_keys_out_of_range() {
    for mk in [0u64, 1, 1022, 1023, u64::MAX] {
        let s = storage_with_meta(meta_page(BTREE_MAGIC, mk));
        assert_eq!(
            io_kind(open_err(s)),
            io::ErrorKind::InvalidData,
            "max_keys {mk}"
        );
    }
}

#[test]
fn open_accepts_hand_built_valid_meta_at_bounds() {
    for mk in [2u64, 1021] {
        let s = storage_with_meta(meta_page(BTREE_MAGIC, mk));
        let mut t = PagedBTree::open(BufferPool::new(Pager::new(s).unwrap(), 2)).unwrap();
        assert_eq!((t.root(), t.height()), (1, 0));
        check(&t, mk as usize);
        for k in 0..10 {
            t.insert(k, k).unwrap();
        }
        check(&t, mk as usize);
    }
}

// ---------------------------------------------------------------- model proptests (I1/I4)

#[derive(Debug, Clone)]
enum Op {
    Insert(u64, u64),
    Get(u64),
    Range(u64, u64),
    RangeRev(u64, u64),
}

fn key() -> impl Strategy<Value = u64> {
    prop_oneof![
        6 => 0u64..200,
        1 => Just(0u64),
        1 => Just(u64::MAX),
        1 => (u64::MAX - 5)..=u64::MAX,
        1 => any::<u64>(),
    ]
}

fn op() -> impl Strategy<Value = Op> {
    prop_oneof![
        6 => (key(), any::<u64>()).prop_map(|(k, v)| Op::Insert(k, v)),
        2 => key().prop_map(Op::Get),
        1 => (key(), key()).prop_map(|(a, b)| Op::Range(a, b)),
        1 => (key(), key()).prop_map(|(a, b)| Op::RangeRev(a, b)),
    ]
}

/// Run `ops` against tree + model; `check()` after every insert.
fn run_model<S: Storage>(t: &mut PagedBTree<S>, mk: usize, ops: &[Op]) -> BTreeMap<u64, u64> {
    let mut m = BTreeMap::new();
    for o in ops {
        match *o {
            Op::Insert(k, v) => {
                assert_eq!(t.insert(k, v).unwrap(), m.insert(k, v), "insert({k},{v})");
                check(t, mk);
            }
            Op::Get(k) => assert_eq!(t.get(k).unwrap(), m.get(&k).copied(), "get({k})"),
            Op::Range(a, b) => {
                assert_eq!(
                    t.range(a, b).unwrap(),
                    model_range(&m, a, b),
                    "range({a},{b})"
                )
            }
            Op::RangeRev(a, b) => assert_eq!(
                t.range_rev(a, b).unwrap(),
                model_range_rev(&m, a, b),
                "range_rev({a},{b})"
            ),
        }
        assert_eq!(t.pool().pinned_frames(), 0, "pin leak after {o:?}");
    }
    assert_matches_model(t, &m);
    m
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(256))]

    #[test]
    fn mem_tree_matches_btreemap(
        mk in 2usize..=6,
        frames in 2usize..=4,
        ops in prop::collection::vec(op(), 0..300),
    ) {
        let mut t = mem_tree(mk, frames);
        run_model(&mut t, mk, &ops);
    }
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(24))]

    #[test]
    fn file_tree_matches_btreemap_and_survives_reopen(
        mk in 2usize..=6,
        frames in 2usize..=4,
        ops in prop::collection::vec(op(), 0..200),
        more in prop::collection::vec(op(), 0..100),
    ) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("p.db");
        let mut t = PagedBTree::create(file_pool(&path, frames), mk).unwrap();
        let mut m = run_model(&mut t, mk, &ops);
        t.pool().flush().unwrap();
        let (root, height) = (t.root(), t.height());
        drop(t);

        let mut t = PagedBTree::open(file_pool(&path, frames)).unwrap();
        prop_assert_eq!((t.root(), t.height()), (root, height));
        check(&t, mk);
        assert_matches_model(&t, &m);
        // Continue after reopen, comparing against the carried-over model.
        for o in &more {
            if let Op::Insert(k, v) = *o {
                prop_assert_eq!(t.insert(k, v).unwrap(), m.insert(k, v));
                check(&t, mk);
            }
        }
        assert_matches_model(&t, &m);
    }
}

// ---------------------------------------------------------------- check() is not fail-open

/// Build a small multi-level tree, apply `corrupt` to one of its pages, expect `check` to panic.
fn check_rejects(corrupt: impl Fn(&BufferPool<MemStorage>, &Shape, PageId)) {
    let mut t = mem_tree(3, 4);
    for k in 0..40u64 {
        t.insert(k * 2, k).unwrap();
    }
    let shape = check(&t, 3);
    corrupt(t.pool(), &shape, t.root());
    let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| check(&t, 3)));
    assert!(r.is_err(), "check() accepted a corrupted tree");
}

fn with_leaf(pool: &BufferPool<MemStorage>, id: PageId, f: impl FnOnce(&mut LeafPage)) {
    let mut g = pool.fetch_mut(id).unwrap();
    f(bytemuck::cast_mut(&mut *g));
}

#[test]
fn check_detects_broken_prev_link() {
    check_rejects(|pool, s, _| with_leaf(pool, s.leaves[2], |l| l.prev = s.leaves[0]));
}

#[test]
fn check_detects_broken_next_link() {
    check_rejects(|pool, s, _| with_leaf(pool, s.leaves[0], |l| l.next = NO_PAGE));
}

#[test]
fn check_detects_unsorted_leaf() {
    check_rejects(|pool, s, _| with_leaf(pool, s.leaves[1], |l| l.keys.swap(0, 1)));
}

#[test]
fn check_detects_key_outside_separator_bounds() {
    check_rejects(|pool, s, _| with_leaf(pool, s.leaves[0], |l| l.keys[0] = 1_000_000));
}

#[test]
fn check_detects_overfull_leaf() {
    // Last leaf has no upper bound: refill it with base, base+1, .., base+3 (sorted,
    // in bounds, count = max_keys + 1) so only the capacity assertion can fire.
    check_rejects(|pool, s, _| {
        with_leaf(pool, *s.leaves.last().unwrap(), |l| {
            let base = l.keys[0];
            for i in 0..4 {
                l.keys[i] = base + i as u64;
            }
            l.count = 4;
        })
    });
}

#[test]
fn check_detects_stale_meta_height() {
    check_rejects(|pool, _, _| {
        let mut g = pool.fetch_mut(0).unwrap();
        let m: &mut MetaPage = bytemuck::cast_mut(&mut *g);
        m.height += 1;
    });
}

#[test]
fn check_detects_shared_child() {
    check_rejects(|pool, _, root| {
        let mut g = pool.fetch_mut(root).unwrap();
        let n: &mut InternalPage = bytemuck::cast_mut(&mut *g);
        n.children[1] = n.children[0];
    });
}

//! Blind contract tests for the buffer pool (stage 2, diff 04).
//!
//! Written from the contract only (spec §2 P1, P2, P5 + the public API brief),
//! without reading the implementation. Every count is asserted exactly.

use std::collections::{HashMap, HashSet};
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::path::Path;

use bytemuck::Zeroable;
use index_lab::storage::file::FileStorage;
use index_lab::storage::pager::Pager;
use index_lab::storage::pool::{BufferPool, PoolError, PoolStats};
use index_lab::storage::{MemStorage, PAGE_SIZE, Page, PageId, Storage};
use proptest::prelude::*;

// ---------------------------------------------------------------- helpers

fn mem_pool(cap: usize) -> BufferPool<MemStorage> {
    BufferPool::new(Pager::new(MemStorage::new()).unwrap(), cap)
}

fn file_pool(path: &Path, cap: usize) -> BufferPool<FileStorage> {
    BufferPool::new(Pager::new(FileStorage::open(path).unwrap()).unwrap(), cap)
}

/// First byte of a page.
fn first(p: &Page) -> u8 {
    p.0[0]
}

/// Last byte of a page (checks that whole pages travel, not a prefix).
fn last(p: &Page) -> u8 {
    p.0[PAGE_SIZE - 1]
}

/// Every byte zero (Page has no PartialEq).
fn is_zeroed(p: &Page) -> bool {
    p.0 == Page::zeroed().0
}

/// Stamp a page: first and last byte = v.
fn stamp(p: &mut Page, v: u8) {
    p.0[0] = v;
    p.0[PAGE_SIZE - 1] = v;
}

fn stats(hits: u64, misses: u64, evictions: u64) -> PoolStats {
    PoolStats {
        hits,
        misses,
        evictions,
    }
}

fn io<S: Storage>(pool: &BufferPool<S>) -> (u64, u64) {
    pool.io_counts()
}

fn err_of<T>(r: Result<T, PoolError>) -> Option<PoolError> {
    r.err()
}

fn is_all_pinned<T>(r: Result<T, PoolError>) -> bool {
    matches!(err_of(r), Some(PoolError::AllPinned))
}

fn borrowed_id<T>(r: Result<T, PoolError>) -> Option<PageId> {
    match err_of(r) {
        Some(PoolError::Borrowed(id)) => Some(id),
        _ => None,
    }
}

/// Create `vals.len()` pages through the pool, stamping each with its value.
/// Guards are dropped immediately. Returns the ids in creation order.
fn make_pages<S: Storage>(pool: &BufferPool<S>, vals: &[u8]) -> Vec<PageId> {
    vals.iter()
        .map(|&v| {
            let (id, mut g) = pool.new_page().unwrap();
            stamp(&mut g, v);
            id
        })
        .collect()
}

/// Write `vals` as fresh pages into a file and close it. Returns ids.
fn seed_file(path: &Path, vals: &[u8]) -> Vec<PageId> {
    let pool = file_pool(path, vals.len().max(1));
    let ids = make_pages(&pool, vals);
    pool.flush().unwrap();
    assert_eq!(io(&pool), (0, vals.len() as u64));
    ids
}

fn resident<S: Storage>(pool: &BufferPool<S>, ids: &[PageId]) -> Vec<bool> {
    ids.iter().map(|&id| pool.is_resident(id)).collect()
}

// ---------------------------------------------------------------- basics

#[test]
fn capacity_reported() {
    assert_eq!(mem_pool(3).capacity(), 3);
}

#[test]
#[should_panic]
fn capacity_zero_panics() {
    let _ = mem_pool(0);
}

#[test]
fn new_page_is_zeroed_dirty_pinned_and_neither_hit_nor_miss() {
    let pool = mem_pool(2);
    let (id, g) = pool.new_page().unwrap();
    assert!(is_zeroed(&g), "new_page frame not zeroed");
    assert_eq!(pool.pinned_frames(), 1);
    drop(g);
    assert_eq!(pool.pinned_frames(), 0);
    assert!(pool.is_resident(id));
    assert!(pool.is_dirty(id));
    assert_eq!(pool.stats(), stats(0, 0, 0));
    assert_eq!(io(&pool), (0, 0), "new_page must not read or write");
}

#[test]
fn new_page_ids_are_distinct() {
    let pool = mem_pool(2);
    let ids = make_pages(&pool, &[1, 2, 3, 4, 5]);
    let set: HashSet<_> = ids.iter().collect();
    assert_eq!(set.len(), 5);
}

#[test]
fn hit_costs_zero_disk_reads() {
    let pool = mem_pool(2);
    let ids = make_pages(&pool, &[10, 20]);
    for _ in 0..3 {
        for &id in &ids {
            let g = pool.fetch(id).unwrap();
            assert_eq!(first(&g), if id == ids[0] { 10 } else { 20 });
        }
    }
    assert_eq!(pool.stats(), stats(6, 0, 0));
    assert_eq!(io(&pool), (0, 0));
}

#[test]
fn miss_reads_once_then_hits() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("p.db");
    let ids = seed_file(&path, &[7, 8]);

    let pool = file_pool(&path, 2);
    assert!(!pool.is_resident(ids[0]));
    {
        let g = pool.fetch(ids[0]).unwrap();
        assert_eq!((first(&g), last(&g)), (7, 7));
    }
    assert_eq!(pool.stats(), stats(0, 1, 0));
    assert_eq!(io(&pool), (1, 0));
    assert!(pool.is_resident(ids[0]));
    assert!(!pool.is_dirty(ids[0]), "a page read from disk starts clean");
    {
        let g = pool.fetch(ids[0]).unwrap();
        assert_eq!(first(&g), 7);
    }
    assert_eq!(pool.stats(), stats(1, 1, 0));
    assert_eq!(io(&pool), (1, 0));
}

#[test]
fn modification_visible_to_later_fetch_in_same_frame() {
    let pool = mem_pool(2);
    let ids = make_pages(&pool, &[1]);
    pool.flush().unwrap();
    {
        let mut g = pool.fetch_mut(ids[0]).unwrap();
        stamp(&mut g, 42);
    }
    let g = pool.fetch(ids[0]).unwrap();
    assert_eq!((first(&g), last(&g)), (42, 42));
    drop(g);
    assert_eq!(pool.stats(), stats(2, 0, 0));
    assert_eq!(io(&pool), (0, 1));
}

#[test]
fn many_page_refs_on_one_page_use_one_frame() {
    let pool = mem_pool(2);
    let ids = make_pages(&pool, &[5]);
    let a = pool.fetch(ids[0]).unwrap();
    let b = pool.fetch(ids[0]).unwrap();
    let c = pool.fetch(ids[0]).unwrap();
    assert_eq!(pool.pinned_frames(), 1, "one page, one frame");
    assert_eq!(first(&a) + first(&b) + first(&c), 15);
    drop((a, b, c));
    assert_eq!(pool.pinned_frames(), 0);
    assert_eq!(pool.stats(), stats(3, 0, 0));
}

// ---------------------------------------------------------------- dirty / clean

#[test]
fn fetch_mut_without_mutation_stays_clean() {
    let pool = mem_pool(1);
    let ids = make_pages(&pool, &[3]);
    pool.flush().unwrap();
    assert_eq!(io(&pool), (0, 1));
    {
        let g = pool.fetch_mut(ids[0]).unwrap();
        assert_eq!(first(&g), 3); // read through Deref only
    }
    assert!(!pool.is_dirty(ids[0]));
    pool.flush().unwrap();
    assert_eq!(io(&pool), (0, 1), "flush of a clean page writes nothing");
    // Evict it: clean victim, no write.
    let _ = make_pages(&pool, &[4]);
    assert_eq!(io(&pool), (0, 1));
    assert_eq!(pool.stats(), stats(1, 0, 1));
}

#[test]
fn fetch_mut_with_mutation_is_dirty_after_drop() {
    let pool = mem_pool(1);
    let ids = make_pages(&pool, &[3]);
    pool.flush().unwrap();
    {
        let mut g = pool.fetch_mut(ids[0]).unwrap();
        stamp(&mut g, 9);
    }
    assert!(pool.is_dirty(ids[0]));
    pool.flush().unwrap();
    assert!(!pool.is_dirty(ids[0]));
    assert_eq!(io(&pool), (0, 2));
}

/// Mutation (a): dirty victim not written back.
#[test]
fn dirty_victim_written_before_reuse() {
    let pool = mem_pool(1);
    let a = make_pages(&pool, &[11])[0];
    assert_eq!(io(&pool), (0, 0));
    let b = make_pages(&pool, &[22])[0];
    assert_eq!(io(&pool), (0, 1), "dirty victim a must be written once");
    assert_eq!(pool.stats(), stats(0, 0, 1));
    assert!(!pool.is_resident(a));
    assert!(pool.is_resident(b));

    // Bring a back: b (dirty) is written, a is read with its stamped bytes.
    let got = pool.fetch(a).map(|g| (first(&g), last(&g)));
    assert_eq!(io(&pool), (1, 2), "dirty victim b must be written once");
    assert!(
        matches!(got, Ok((11, 11))),
        "a's content lost: {:?}",
        got.ok()
    );
    assert_eq!(pool.stats(), stats(0, 1, 2));
}

/// Mutation (a), second form: a page that is already on disk is modified,
/// then evicted. The new bytes, not the old disk bytes, must come back.
#[test]
fn modified_page_survives_eviction() {
    let pool = mem_pool(1);
    let a = make_pages(&pool, &[1])[0];
    pool.flush().unwrap();
    {
        let mut g = pool.fetch_mut(a).unwrap();
        stamp(&mut g, 99);
    }
    let _b = make_pages(&pool, &[2]);
    assert_eq!(io(&pool), (0, 2));
    let got = pool.fetch(a).map(|g| (first(&g), last(&g)));
    assert!(matches!(got, Ok((99, 99))), "got {:?}", got.ok());
    assert_eq!(io(&pool), (1, 3));
}

#[test]
fn clean_victim_not_written() {
    let pool = mem_pool(1);
    let a = make_pages(&pool, &[1])[0];
    pool.flush().unwrap();
    assert_eq!(io(&pool), (0, 1));
    {
        let g = pool.fetch(a).unwrap();
        assert_eq!(first(&g), 1);
    }
    let _b = make_pages(&pool, &[2]);
    assert_eq!(io(&pool), (0, 1), "clean victim must not be written");
    assert_eq!(pool.stats(), stats(1, 0, 1));
}

#[test]
fn flush_writes_every_dirty_once_then_nothing() {
    let pool = mem_pool(4);
    let ids = make_pages(&pool, &[1, 2, 3]);
    pool.flush().unwrap();
    assert_eq!(io(&pool), (0, 3));
    for &id in &ids {
        assert!(!pool.is_dirty(id));
        assert!(pool.is_resident(id), "flush does not evict");
    }
    pool.flush().unwrap();
    assert_eq!(io(&pool), (0, 3), "second flush writes 0");

    // Dirty only one of them; flush writes exactly one.
    {
        let mut g = pool.fetch_mut(ids[1]).unwrap();
        stamp(&mut g, 50);
    }
    pool.flush().unwrap();
    assert_eq!(io(&pool), (0, 4));
    assert_eq!(pool.stats(), stats(1, 0, 0));
}

#[test]
fn flush_with_live_page_mut_on_dirty_page_is_borrowed() {
    let pool = mem_pool(2);
    let (id, mut g) = pool.new_page().unwrap();
    stamp(&mut g, 5);
    // new_page's page is dirty and its PageMut is alive.
    assert_eq!(borrowed_id(pool.flush()), Some(id));
    drop(g);
    assert_eq!(pool.pinned_frames(), 0);
    pool.flush().unwrap();
    assert_eq!(io(&pool), (0, 1));
    assert!(!pool.is_dirty(id));

    // Dirty page, then a live PageMut on it.
    {
        let mut g = pool.fetch_mut(id).unwrap();
        stamp(&mut g, 6);
    }
    let g = pool.fetch_mut(id).unwrap();
    assert_eq!(borrowed_id(pool.flush()), Some(id));
    drop(g);
    pool.flush().unwrap();
    assert_eq!(io(&pool), (0, 2));
}

#[test]
fn flush_with_live_page_mut_on_clean_page_is_ok() {
    let pool = mem_pool(2);
    let ids = make_pages(&pool, &[1, 2]);
    pool.flush().unwrap();
    {
        let mut g = pool.fetch_mut(ids[1]).unwrap();
        stamp(&mut g, 3);
    }
    let held = pool.fetch_mut(ids[0]).unwrap(); // clean, nothing to write
    pool.flush().unwrap();
    assert_eq!(io(&pool), (0, 3));
    assert!(!pool.is_dirty(ids[1]));
    drop(held);
}

#[test]
fn flush_with_live_page_ref_on_dirty_page_is_ok() {
    let pool = mem_pool(2);
    let ids = make_pages(&pool, &[7]);
    let g = pool.fetch(ids[0]).unwrap();
    pool.flush().unwrap();
    assert_eq!(io(&pool), (0, 1));
    assert_eq!(first(&g), 7);
    drop(g);
    assert!(!pool.is_dirty(ids[0]));
}

// ---------------------------------------------------------------- LRU

/// Mutation (b): MRU instead of LRU.
#[test]
fn lru_evicts_oldest_created() {
    let pool = mem_pool(3);
    let ids = make_pages(&pool, &[1, 2, 3]); // use order a, b, c
    let d = make_pages(&pool, &[4])[0];
    assert_eq!(resident(&pool, &ids), vec![false, true, true]);
    assert!(pool.is_resident(d));
    assert_eq!(pool.stats(), stats(0, 0, 1));
}

#[test]
fn lru_refetch_reorders() {
    let pool = mem_pool(3);
    let v = make_pages(&pool, &[1, 2, 3]); // t: a1 b2 c3
    let (a, b, c) = (v[0], v[1], v[2]);
    drop(pool.fetch(a).unwrap()); // a4 -> order b, c, a
    let d = make_pages(&pool, &[4])[0]; // d5, evicts b
    assert_eq!(
        resident(&pool, &[a, b, c, d]),
        vec![true, false, true, true]
    );
    drop(pool.fetch(c).unwrap()); // c6
    drop(pool.fetch_mut(a).unwrap()); // a7 (fetch_mut is a use too)
    let e = make_pages(&pool, &[5])[0]; // e8, evicts d (d5 < c6 < a7)
    assert_eq!(
        resident(&pool, &[a, b, c, d, e]),
        vec![true, false, true, false, true]
    );
    // b and d were dirty victims: 2 writes, nothing read.
    assert_eq!(io(&pool), (0, 2));
    assert_eq!(pool.stats(), stats(3, 0, 2));

    // Re-fetch b: evicts the LRU among c6, a7, e8 = c. Read 1, write 1 (c dirty).
    let got = pool.fetch(b).map(|g| first(&g));
    assert!(matches!(got, Ok(2)));
    assert_eq!(
        resident(&pool, &[a, b, c, d, e]),
        vec![true, true, false, false, true]
    );
    assert_eq!(io(&pool), (1, 3));
    assert_eq!(pool.stats(), stats(3, 1, 3));
}

#[test]
fn lru_over_pages_loaded_by_miss() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("lru.db");
    let p = seed_file(&path, &[10, 11, 12]);

    let pool = file_pool(&path, 2);
    drop(pool.fetch(p[0]).unwrap()); // miss, t1
    drop(pool.fetch(p[1]).unwrap()); // miss, t2
    drop(pool.fetch(p[0]).unwrap()); // hit,  t3
    drop(pool.fetch(p[2]).unwrap()); // miss, evicts p1 (t2)
    assert_eq!(resident(&pool, &p), vec![true, false, true]);
    assert_eq!(pool.stats(), stats(1, 3, 1));
    assert_eq!(io(&pool), (3, 0));

    let got = pool.fetch(p[1]).map(|g| first(&g)); // evicts p0 (t3 < t4)
    assert!(matches!(got, Ok(11)));
    assert_eq!(resident(&pool, &p), vec![false, true, true]);
    assert_eq!(pool.stats(), stats(1, 4, 2));
    assert_eq!(io(&pool), (4, 0), "all victims were clean");
}

#[test]
fn unpin_is_not_a_use() {
    let pool = mem_pool(2);
    let v = make_pages(&pool, &[1, 2]);
    pool.flush().unwrap();
    let (a, b) = (v[0], v[1]);
    let ga = pool.fetch(a).unwrap(); // a pinned at t3
    let gb = pool.fetch(b).unwrap(); // b pinned at t4
    drop(gb); // b unpinned first
    drop(ga); // a unpinned last, but last *pinned* earlier
    let c = make_pages(&pool, &[3])[0];
    assert_eq!(resident(&pool, &[a, b, c]), vec![false, true, true]);
}

// ---------------------------------------------------------------- pins

/// Mutation (c): pinned frames treated as evictable.
#[test]
fn pinned_lru_page_is_skipped() {
    let pool = mem_pool(2);
    let v = make_pages(&pool, &[1, 2]);
    pool.flush().unwrap();
    let (a, b) = (v[0], v[1]);
    let ga = pool.fetch(a).unwrap(); // a: t3, pinned
    drop(pool.fetch(b).unwrap()); // b: t4, unpinned
    // a is the LRU but pinned; b must be the victim.
    let r = catch_unwind(AssertUnwindSafe(|| pool.new_page().map(|(id, _g)| id)));
    assert!(
        r.is_ok(),
        "new_page panicked: a pinned frame was chosen as victim?"
    );
    let c = r.unwrap().unwrap();
    assert!(pool.is_resident(a), "pinned page a was evicted");
    assert!(!pool.is_resident(b));
    assert!(pool.is_resident(c));
    assert_eq!(
        (first(&ga), last(&ga)),
        (1, 1),
        "pinned frame contents changed"
    );
    assert_eq!(pool.pinned_frames(), 1);
    drop(ga);
    assert_eq!(pool.pinned_frames(), 0);
}

/// Mutation (c), fetch path, with a dirty pinned page.
#[test]
fn pinned_dirty_page_not_evicted_by_fetch() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("pin.db");
    let p = seed_file(&path, &[1, 2, 3]);
    let pool = file_pool(&path, 2);

    let mut g0 = pool.fetch_mut(p[0]).unwrap(); // t1, pinned
    stamp(&mut g0, 100);
    drop(pool.fetch(p[1]).unwrap()); // t2
    let r = catch_unwind(AssertUnwindSafe(|| pool.fetch(p[2]).map(|g| first(&g))));
    assert!(r.is_ok(), "fetch panicked: pinned frame chosen as victim?");
    assert!(matches!(r.unwrap(), Ok(3)));
    assert_eq!(resident(&pool, &p), vec![true, false, true]);
    assert_eq!(io(&pool), (3, 0), "clean p1 evicted; pinned p0 not written");
    assert_eq!(first(&g0), 100);
    drop(g0);
    assert!(pool.is_dirty(p[0]));
}

#[test]
fn all_pinned_is_err_and_recovers() {
    let pool = mem_pool(2);
    let v = make_pages(&pool, &[1, 2, 3]); // v[0] evicted to disk
    let (x, a, b) = (v[0], v[1], v[2]);
    let ga = pool.fetch(a).unwrap();
    let gb = pool.fetch_mut(b).unwrap();
    assert_eq!(pool.pinned_frames(), 2);
    let io_before = io(&pool);

    assert!(is_all_pinned(pool.fetch(x)), "fetch must be Err(AllPinned)");
    assert!(
        is_all_pinned(pool.fetch_mut(x)),
        "fetch_mut must be Err(AllPinned)"
    );
    assert!(
        is_all_pinned(pool.new_page()),
        "new_page must be Err(AllPinned)"
    );
    assert_eq!(pool.pinned_frames(), 2, "failed calls must not pin");
    assert!(!pool.is_resident(x));
    assert_eq!(io(&pool), io_before, "no frame -> no disk I/O");
    // Resident pages are still reachable while everything is pinned.
    assert_eq!(first(&pool.fetch(a).unwrap()), 2);

    drop(gb);
    let got = pool.fetch(x).map(|g| first(&g));
    assert!(matches!(got, Ok(1)), "after unpin, fetch must succeed");
    assert!(pool.is_resident(a), "a is still pinned");
    assert!(!pool.is_resident(b));
    drop(ga);
    assert_eq!(pool.pinned_frames(), 0);
}

#[test]
fn all_pinned_capacity_one() {
    let pool = mem_pool(1);
    let (a, g) = pool.new_page().unwrap();
    assert!(is_all_pinned(pool.new_page()));
    assert_eq!(pool.pinned_frames(), 1);
    drop(g);
    let b = make_pages(&pool, &[1])[0];
    assert!(!pool.is_resident(a));
    assert!(pool.is_resident(b));
    assert_eq!(io(&pool), (0, 1));
}

// ---------------------------------------------------------------- conflicts

#[test]
fn conflicting_guards_are_borrowed_and_leak_no_pin() {
    let pool = mem_pool(1);
    let a = make_pages(&pool, &[1])[0];

    let r1 = pool.fetch(a).unwrap();
    assert_eq!(
        borrowed_id(pool.fetch_mut(a)),
        Some(a),
        "PageMut while PageRef"
    );
    let r2 = pool.fetch(a).unwrap(); // multiple PageRef is fine
    assert_eq!(borrowed_id(pool.fetch_mut(a)), Some(a));
    assert_eq!(pool.pinned_frames(), 1);
    drop(r1);
    assert_eq!(borrowed_id(pool.fetch_mut(a)), Some(a), "still one PageRef");
    drop(r2);
    assert_eq!(pool.pinned_frames(), 0, "Borrowed errors leaked a pin");

    let m = pool.fetch_mut(a).unwrap();
    assert_eq!(borrowed_id(pool.fetch(a)), Some(a), "PageRef while PageMut");
    assert_eq!(
        borrowed_id(pool.fetch_mut(a)),
        Some(a),
        "PageMut while PageMut"
    );
    assert_eq!(pool.pinned_frames(), 1);
    drop(m);
    assert_eq!(pool.pinned_frames(), 0, "Borrowed errors leaked a pin");

    // The only frame must be reusable now: a leaked pin would give AllPinned.
    let r = pool.new_page().map(|(id, _g)| id);
    assert!(r.is_ok(), "frame still pinned after conflicts");
    assert!(!pool.is_resident(a));
}

#[test]
fn page_mut_from_new_page_conflicts() {
    let pool = mem_pool(2);
    let (id, g) = pool.new_page().unwrap();
    assert_eq!(borrowed_id(pool.fetch(id)), Some(id));
    assert_eq!(borrowed_id(pool.fetch_mut(id)), Some(id));
    assert_eq!(pool.pinned_frames(), 1);
    drop(g);
    assert_eq!(pool.pinned_frames(), 0);
}

#[test]
fn fetch_of_unallocated_id_is_io_error_without_pin() {
    let pool = mem_pool(2);
    let _ = make_pages(&pool, &[1]);
    pool.flush().unwrap();
    let r = pool.fetch(9_999);
    assert!(matches!(err_of(r), Some(PoolError::Io(_))));
    let r = pool.fetch_mut(9_999);
    assert!(matches!(err_of(r), Some(PoolError::Io(_))));
    assert_eq!(pool.pinned_frames(), 0);
    assert!(!pool.is_resident(9_999));
    // Both frames still usable.
    let _ = make_pages(&pool, &[2, 3]);
    assert_eq!(pool.pinned_frames(), 0);
}

// ---------------------------------------------------------------- P1

/// One fixed op sequence; returns contents of every page plus counters.
fn script<S: Storage>(pool: &BufferPool<S>) -> (Vec<(u8, u8)>, (u64, u64), PoolStats) {
    let ids = make_pages(pool, &[1, 2, 3, 4, 5]); // 3 evictions (dirty)
    pool.flush().unwrap();
    for (i, &id) in ids.iter().enumerate() {
        if i % 2 == 0 {
            let mut g = pool.fetch_mut(id).unwrap();
            let v = first(&g);
            stamp(&mut g, v + 100);
        } else {
            drop(pool.fetch(id).unwrap());
        }
    }
    drop(pool.fetch_mut(ids[1]).unwrap()); // read-only PageMut
    pool.flush().unwrap();
    let contents = ids
        .iter()
        .map(|&id| {
            let g = pool.fetch(id).unwrap();
            (first(&g), last(&g))
        })
        .collect();
    (contents, io(pool), pool.stats())
}

#[test]
fn mem_and_file_storage_agree() {
    let dir = tempfile::tempdir().unwrap();
    let mem = script(&mem_pool(2));
    let file = script(&file_pool(&dir.path().join("p1.db"), 2));
    assert_eq!(mem, file);
    assert_eq!(
        mem.0,
        vec![(101, 101), (2, 2), (103, 103), (4, 4), (105, 105)]
    );
}

// ---------------------------------------------------------------- P5

#[test]
fn flush_close_reopen_keeps_contents() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("p5.db");
    let ids = {
        let pool = file_pool(&path, 2);
        let ids = make_pages(&pool, &[1, 2, 3, 4]);
        {
            let mut g = pool.fetch_mut(ids[0]).unwrap(); // evicted earlier -> miss
            stamp(&mut g, 201);
        }
        pool.flush().unwrap();
        ids
    };
    let pool = file_pool(&path, 2);
    assert_eq!(pool.io_counts().0, 0, "fresh pager counters");
    let got: Vec<u8> = ids
        .iter()
        .map(|&id| first(&pool.fetch(id).unwrap()))
        .collect();
    assert_eq!(got, vec![201, 2, 3, 4]);
    assert_eq!(pool.stats(), stats(0, 4, 2));
    assert_eq!(io(&pool), (4, 0));
    // Allocation continues past the existing pages.
    let n = make_pages(&pool, &[9])[0];
    assert!(!ids.contains(&n));
}

#[test]
fn without_flush_resident_modifications_are_not_on_disk() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("noflush.db");
    let p = seed_file(&path, &[1, 2, 3]);
    {
        let pool = file_pool(&path, 2);
        for (&id, v) in p.iter().zip([51u8, 52, 53]) {
            let mut g = pool.fetch_mut(id).unwrap();
            stamp(&mut g, v);
        }
        // p0 was evicted (dirty, written); p1, p2 resident and dirty.
        assert_eq!(io(&pool), (3, 1));
        assert!(pool.is_dirty(p[1]) && pool.is_dirty(p[2]));
        // dropped without flush
    }
    let pool = file_pool(&path, 3);
    let got: Vec<u8> = p
        .iter()
        .map(|&id| first(&pool.fetch(id).unwrap()))
        .collect();
    assert_eq!(got, vec![51, 2, 3], "only the evicted page reached disk");
    assert_eq!(io(&pool), (3, 0));
}

// ---------------------------------------------------------------- model proptest

#[derive(Debug, Clone)]
enum Op {
    New(u8),
    Fetch(usize),
    /// fetch_mut; Some(v) = mutate to v, None = read only
    FetchMut(usize, Option<u8>),
    Flush,
    /// Hold a PageRef on page i while fetching page j.
    PinFetch(usize, usize),
    /// Hold a PageRef on page i while creating a page.
    PinNew(usize, u8),
}

fn op() -> impl Strategy<Value = Op> {
    prop_oneof![
        2 => any::<u8>().prop_map(Op::New),
        4 => any::<usize>().prop_map(Op::Fetch),
        3 => (any::<usize>(), proptest::option::of(any::<u8>()))
            .prop_map(|(i, v)| Op::FetchMut(i, v)),
        1 => Just(Op::Flush),
        2 => (any::<usize>(), any::<usize>()).prop_map(|(i, j)| Op::PinFetch(i, j)),
        1 => (any::<usize>(), any::<u8>()).prop_map(|(i, v)| Op::PinNew(i, v)),
    ]
}

/// Reference model of an LRU, pin-aware, write-back pool.
struct Model {
    cap: usize,
    ids: Vec<PageId>,
    value: HashMap<PageId, u8>,
    disk: HashMap<PageId, u8>,
    last_use: HashMap<PageId, u64>, // resident pages only
    dirty: HashSet<PageId>,
    clock: u64,
    reads: u64,
    writes: u64,
    hits: u64,
    misses: u64,
    evictions: u64,
}

impl Model {
    fn new(cap: usize) -> Self {
        Model {
            cap,
            ids: vec![],
            value: HashMap::new(),
            disk: HashMap::new(),
            last_use: HashMap::new(),
            dirty: HashSet::new(),
            clock: 0,
            reads: 0,
            writes: 0,
            hits: 0,
            misses: 0,
            evictions: 0,
        }
    }

    /// Make room for one page. Err = all pinned.
    fn make_room(&mut self, pinned: Option<PageId>) -> Result<(), ()> {
        if self.last_use.len() < self.cap {
            return Ok(());
        }
        let victim = self
            .last_use
            .iter()
            .filter(|(id, _)| Some(**id) != pinned)
            .min_by_key(|(_, t)| **t)
            .map(|(id, _)| *id)
            .ok_or(())?;
        self.last_use.remove(&victim);
        if self.dirty.remove(&victim) {
            self.writes += 1;
            self.disk.insert(victim, self.value[&victim]);
        }
        self.evictions += 1;
        Ok(())
    }

    fn touch(&mut self, id: PageId) {
        self.clock += 1;
        self.last_use.insert(id, self.clock);
    }

    /// Model a fetch of id; Err = all pinned.
    fn fetch(&mut self, id: PageId, pinned: Option<PageId>) -> Result<(), ()> {
        if self.last_use.contains_key(&id) {
            self.hits += 1;
        } else {
            self.make_room(pinned)?;
            self.misses += 1;
            self.reads += 1;
            assert_eq!(self.disk.get(&id), self.value.get(&id), "model: disk stale");
        }
        self.touch(id);
        Ok(())
    }

    fn new_page(&mut self, id: PageId, v: u8) {
        self.ids.push(id);
        self.value.insert(id, v);
        self.dirty.insert(id);
        self.touch(id);
    }

    fn flush(&mut self) {
        for id in self.dirty.drain() {
            self.writes += 1;
            self.disk.insert(id, self.value[&id]);
        }
    }
}

fn check_pool<S: Storage>(pool: &BufferPool<S>, m: &Model) -> Result<(), TestCaseError> {
    prop_assert_eq!(pool.pinned_frames(), 0);
    prop_assert_eq!(io(pool), (m.reads, m.writes));
    prop_assert_eq!(pool.stats(), stats(m.hits, m.misses, m.evictions));
    let mut n_resident = 0;
    for &id in &m.ids {
        let r = pool.is_resident(id);
        prop_assert_eq!(r, m.last_use.contains_key(&id), "resident {}", id);
        if r {
            n_resident += 1;
            prop_assert_eq!(pool.is_dirty(id), m.dirty.contains(&id), "dirty {}", id);
        }
    }
    prop_assert!(n_resident <= pool.capacity());
    Ok(())
}

fn run_model<S: Storage>(pool: BufferPool<S>, cap: usize, ops: &[Op]) -> Result<(), TestCaseError> {
    let mut m = Model::new(cap);
    for op in ops {
        let pick = |i: usize| (!m.ids.is_empty()).then(|| m.ids[i % m.ids.len()]);
        match op.clone() {
            Op::New(v) => {
                let (id, mut g) = pool.new_page().map_err(|e| fail(&e))?;
                prop_assert!(is_zeroed(&g), "new_page frame not zeroed");
                stamp(&mut g, v);
                drop(g);
                m.make_room(None).unwrap();
                m.new_page(id, v);
            }
            Op::Fetch(i) => {
                let Some(id) = pick(i) else { continue };
                let g = pool.fetch(id).map_err(|e| fail(&e))?;
                prop_assert_eq!((first(&g), last(&g)), (m.value[&id], m.value[&id]));
                drop(g);
                m.fetch(id, None).unwrap();
            }
            Op::FetchMut(i, v) => {
                let Some(id) = pick(i) else { continue };
                let mut g = pool.fetch_mut(id).map_err(|e| fail(&e))?;
                prop_assert_eq!(first(&g), m.value[&id]);
                if let Some(v) = v {
                    stamp(&mut g, v);
                }
                drop(g);
                m.fetch(id, None).unwrap();
                if let Some(v) = v {
                    m.value.insert(id, v);
                    m.dirty.insert(id);
                }
            }
            Op::Flush => {
                pool.flush().map_err(|e| fail(&e))?;
                m.flush();
            }
            Op::PinFetch(i, j) => {
                let (Some(a), Some(b)) = (pick(i), pick(j)) else {
                    continue;
                };
                let ga = pool.fetch(a).map_err(|e| fail(&e))?;
                m.fetch(a, None).unwrap();
                let r = pool.fetch(b).map(|g| first(&g));
                match m.fetch(b, Some(a)) {
                    Ok(()) => prop_assert_eq!(r.ok(), Some(m.value[&b])),
                    Err(()) => prop_assert!(is_all_pinned(r)),
                }
                prop_assert_eq!(first(&ga), m.value[&a]);
                drop(ga);
            }
            Op::PinNew(i, v) => {
                let Some(a) = pick(i) else { continue };
                let ga = pool.fetch(a).map_err(|e| fail(&e))?;
                m.fetch(a, None).unwrap();
                let r = pool.new_page().map(|(id, mut g)| {
                    stamp(&mut g, v);
                    id
                });
                match m.make_room(Some(a)) {
                    Ok(()) => {
                        let id = r.map_err(|e| fail(&e))?;
                        m.new_page(id, v);
                    }
                    Err(()) => prop_assert!(is_all_pinned(r)),
                }
                drop(ga);
            }
        }
        check_pool(&pool, &m)?;
    }
    // Final: flush, every page's contents equal the model.
    pool.flush().map_err(|e| fail(&e))?;
    m.flush();
    prop_assert_eq!(pool.io_counts().1, m.writes);
    for &id in &m.ids {
        let g = pool.fetch(id).map_err(|e| fail(&e))?;
        prop_assert_eq!((first(&g), last(&g)), (m.value[&id], m.value[&id]));
    }
    Ok(())
}

fn fail(e: &PoolError) -> TestCaseError {
    TestCaseError::fail(format!("unexpected pool error: {e:?}"))
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(192))]

    #[test]
    fn pool_matches_model_mem(cap in 1usize..=4, ops in proptest::collection::vec(op(), 0..48)) {
        run_model(mem_pool(cap), cap, &ops)?;
    }
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(24))]

    #[test]
    fn pool_matches_model_file(cap in 1usize..=4, ops in proptest::collection::vec(op(), 0..32)) {
        let dir = tempfile::tempdir().unwrap();
        run_model(file_pool(&dir.path().join("m.db"), cap), cap, &ops)?;
    }
}

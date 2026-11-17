//! Blind contract tests for the paged Extendible Hash (stage 2, diff 11).
//!
//! Written from the contract only (stage-2 spec §2 P3–P6 + S3, stage-1 I1/I3/I4, the page
//! layout in `hash_page.rs` and the public API brief), without reading the implementation.
//! Besides calling `PagedHash::check()`, the tests walk the pages themselves (`layout` /
//! `verify`) so a fail-open `check()` cannot hide a broken structure, and the
//! `check_detects_*` tests make sure `check()` itself is not fail-open.

use std::collections::{BTreeMap, BTreeSet, HashSet};
use std::io;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::path::Path;

use bytemuck::Zeroable;
use index_lab::index::Index;
use index_lab::index::btree_page::BTREE_MAGIC;
use index_lab::index::hash_page::{
    BUCKET_CAPACITY, BucketPage, DIR_SLOTS, DirectoryPage, HASH_MAGIC, HashMetaPage, MAX_DIR_PAGES,
};
use index_lab::index::paged_btree::PagedBTree;
use index_lab::index::paged_hash::PagedHash;
use index_lab::storage::file::FileStorage;
use index_lab::storage::pager::Pager;
use index_lab::storage::pool::{BufferPool, PoolError};
use index_lab::storage::{MemStorage, Page, PageId, Storage};
use proptest::prelude::*;

// ---------------------------------------------------------------- hashing helpers

/// Copy of the crate-private `index::hash::mix` (splitmix64 finalizer, a bijection).
fn mix(key: u64) -> u64 {
    let mut z = key;
    z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    z ^ (z >> 31)
}

/// Multiplicative inverse of an odd number mod 2^64 (Newton iteration).
fn inv(a: u64) -> u64 {
    let mut x = a;
    for _ in 0..6 {
        x = x.wrapping_mul(2u64.wrapping_sub(a.wrapping_mul(x)));
    }
    x
}

/// Inverse of `z = x ^ (x >> s)`.
fn unxorshift(z: u64, s: u32) -> u64 {
    let mut x = z;
    for _ in 0..(64 / s + 1) {
        x = z ^ (x >> s);
    }
    x
}

/// Inverse of `mix`: the key whose hash is `h`. Lets tests pick exact directory slots.
fn unmix(h: u64) -> u64 {
    let mut x = unxorshift(h, 31);
    x = x.wrapping_mul(inv(0x94d0_49bb_1331_11eb));
    x = unxorshift(x, 27);
    x = x.wrapping_mul(inv(0xbf58_476d_1ce4_e5b9));
    unxorshift(x, 30)
}

fn low(h: u64, bits: u64) -> u64 {
    if bits >= 64 {
        h
    } else {
        h & ((1u64 << bits) - 1)
    }
}

/// Deterministic pseudo-random sequence (splitmix64) for the non-proptest bulk tests.
fn splitmix(seed: &mut u64) -> u64 {
    *seed = seed.wrapping_add(0x9E37_79B9_7F4A_7C15);
    mix(*seed)
}

// ---------------------------------------------------------------- pool helpers

fn mem_pool(cap: usize) -> BufferPool<MemStorage> {
    BufferPool::new(Pager::new(MemStorage::new()).unwrap(), cap)
}

fn file_pool(path: &Path, cap: usize) -> BufferPool<FileStorage> {
    BufferPool::new(Pager::new(FileStorage::open(path).unwrap()).unwrap(), cap)
}

fn mem_hash(bucket_cap: usize, frames: usize) -> PagedHash<MemStorage> {
    PagedHash::create(mem_pool(frames), bucket_cap).unwrap()
}

/// Copy a page out of the pool so the pin is released before the caller fetches more.
fn read_page<S: Storage>(pool: &BufferPool<S>, id: PageId) -> Page {
    let g = pool.fetch(id).unwrap();
    *g
}

fn ceil_dirs(g: u64) -> u64 {
    (1u64 << g).div_ceil(DIR_SLOTS as u64)
}

// ---------------------------------------------------------------- independent page walk

#[derive(Debug, Clone, PartialEq, Eq)]
struct Bucket {
    local_depth: u64,
    keys: Vec<u64>,
    values: Vec<u64>,
    /// Directory slots pointing at this bucket, ascending.
    slots: Vec<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Layout {
    global_depth: u64,
    bucket_cap: u64,
    dir_pages: Vec<PageId>,
    /// slot -> bucket page id, length 2^global_depth.
    slots: Vec<PageId>,
    buckets: BTreeMap<PageId, Bucket>,
}

impl Layout {
    fn pairs(&self) -> Vec<(u64, u64)> {
        let mut v: Vec<(u64, u64)> = self
            .buckets
            .values()
            .flat_map(|b| b.keys.iter().copied().zip(b.values.iter().copied()))
            .collect();
        v.sort_unstable();
        v
    }

    /// Bucket page holding `key` according to the directory.
    fn bucket_of(&self, key: u64) -> PageId {
        self.slots[low(mix(key), self.global_depth) as usize]
    }
}

/// Read the whole structure through `pool.fetch`, one pin at a time.
fn layout<S: Storage>(pool: &BufferPool<S>) -> Layout {
    let meta_p = read_page(pool, 0);
    let meta: &HashMetaPage = bytemuck::cast_ref(&meta_p);
    assert_eq!(meta.magic, HASH_MAGIC, "meta magic");
    let g = meta.global_depth;
    assert!(g <= 21, "global_depth {g} > 21");
    assert_eq!(
        meta.dir_count,
        ceil_dirs(g),
        "dir_count for global_depth {g}"
    );
    let dir_pages = meta.dir_pages().to_vec();
    let n = 1u64 << g;
    let mut slots = Vec::with_capacity(n as usize);
    for (i, &dp) in dir_pages.iter().enumerate() {
        let page = read_page(pool, dp);
        let d: &DirectoryPage = bytemuck::cast_ref(&page);
        let used = (n - i as u64 * DIR_SLOTS as u64).min(DIR_SLOTS as u64) as usize;
        slots.extend_from_slice(&d.slots[..used]);
    }
    assert_eq!(slots.len() as u64, n);
    let mut buckets = BTreeMap::new();
    for (s, &b) in slots.iter().enumerate() {
        buckets
            .entry(b)
            .or_insert_with(|| {
                let p = read_page(pool, b);
                let bp: &BucketPage = bytemuck::cast_ref(&p);
                assert!(
                    bp.count as usize <= BUCKET_CAPACITY,
                    "bucket {b} count {} beyond page capacity",
                    bp.count
                );
                Bucket {
                    local_depth: bp.local_depth,
                    keys: bp.keys().to_vec(),
                    values: bp.values().to_vec(),
                    slots: Vec::new(),
                }
            })
            .slots
            .push(s as u64);
    }
    Layout {
        global_depth: g,
        bucket_cap: meta.bucket_cap,
        dir_pages,
        slots,
        buckets,
    }
}

/// Independent structural check (P4 / I3 + S3 meta consistency), then `PagedHash::check()`.
fn verify<S: Storage>(h: &PagedHash<S>) -> Layout {
    let pool = h.pool();
    assert_eq!(pool.pinned_frames(), 0, "pin leaked before verify");
    let l = layout(pool);
    let g = l.global_depth;
    assert_eq!(
        g,
        h.global_depth() as u64,
        "meta.global_depth != global_depth()"
    );
    assert_eq!(l.bucket_cap, h.bucket_cap() as u64, "meta.bucket_cap");
    let dirs: HashSet<PageId> = l.dir_pages.iter().copied().collect();
    assert_eq!(dirs.len(), l.dir_pages.len(), "directory page listed twice");
    assert!(!dirs.contains(&0), "page 0 used as a directory page");
    let mut all_keys = HashSet::new();
    for (&id, b) in &l.buckets {
        assert_ne!(id, 0, "a slot points at the meta page");
        assert!(!dirs.contains(&id), "a slot points at directory page {id}");
        let d = b.local_depth;
        assert!(d <= g, "bucket {id} local depth {d} > global {g}");
        assert_eq!(
            b.slots.len() as u64,
            1u64 << (g - d),
            "bucket {id}: slot count != 2^(g-d)"
        );
        let suffix = low(b.slots[0], d);
        for &s in &b.slots {
            assert_eq!(
                low(s, d),
                suffix,
                "bucket {id}: slots disagree on low {d} bits"
            );
        }
        assert!(
            b.keys.len() as u64 <= l.bucket_cap,
            "bucket {id} holds {} > bucket_cap {}",
            b.keys.len(),
            l.bucket_cap
        );
        for &k in &b.keys {
            assert_eq!(
                low(mix(k), d),
                suffix,
                "key {k} in bucket {id} with wrong suffix"
            );
            assert!(all_keys.insert(k), "key {k} stored twice");
        }
    }
    assert_eq!(h.bucket_count().unwrap(), l.buckets.len(), "bucket_count()");
    assert_eq!(pool.pinned_frames(), 0, "verify leaked a pin");
    h.check();
    assert_eq!(pool.pinned_frames(), 0, "check() leaked a pin");
    l
}

// ---------------------------------------------------------------- model helpers

fn model_range(m: &BTreeMap<u64, u64>, lo: u64, hi: u64) -> Vec<(u64, u64)> {
    if lo > hi {
        return Vec::new();
    }
    m.range(lo..=hi).map(|(&k, &v)| (k, v)).collect()
}

fn model_range_rev(m: &BTreeMap<u64, u64>, lo: u64, hi: u64) -> Vec<(u64, u64)> {
    let mut v = model_range(m, lo, hi);
    v.reverse();
    v
}

/// Full comparison against the model: every key, neighbours, full ranges, page contents.
fn assert_matches_model<S: Storage>(h: &PagedHash<S>, m: &BTreeMap<u64, u64>) {
    for (&k, &v) in m {
        assert_eq!(h.get(k).unwrap(), Some(v), "get({k})");
        if let Some(k1) = k.checked_add(1)
            && !m.contains_key(&k1)
        {
            assert_eq!(h.get(k1).unwrap(), None, "get({k1}) absent");
        }
    }
    let all: Vec<_> = m.iter().map(|(&k, &v)| (k, v)).collect();
    assert_eq!(h.range(0, u64::MAX).unwrap(), all, "full range");
    let mut rev = all.clone();
    rev.reverse();
    assert_eq!(h.range_rev(0, u64::MAX).unwrap(), rev, "full range_rev");
    assert_eq!(h.pool().pinned_frames(), 0);
    assert_eq!(layout(h.pool()).pairs(), all, "page contents != model");
}

fn io_kind(e: PoolError) -> io::ErrorKind {
    match e {
        PoolError::Io(e) => e.kind(),
        other => panic!("expected PoolError::Io, got {other:?}"),
    }
}

fn panic_message(r: std::thread::Result<()>) -> String {
    let e = r.expect_err("expected a panic");
    if let Some(s) = e.downcast_ref::<String>() {
        s.clone()
    } else if let Some(s) = e.downcast_ref::<&str>() {
        s.to_string()
    } else {
        String::new()
    }
}

// ---------------------------------------------------------------- hand-built storages

/// A valid hash image: meta (global depth `g`, `bucket_cap`), `ceil(2^g / 2048)` directory
/// pages (pages 1..), and buckets assigned by `bucket_of_slot` (bucket index -> local depth,
/// keys). `edit` may then change meta fields.
struct Image {
    g: u64,
    bucket_cap: u64,
    /// (local_depth, keys) per bucket.
    buckets: Vec<(u64, Vec<u64>)>,
    /// slot -> index into `buckets`.
    slot_bucket: fn(u64) -> usize,
}

fn build_storage(img: &Image, edit: impl FnOnce(&mut HashMetaPage)) -> MemStorage {
    let mut s = MemStorage::new();
    let dirs = ceil_dirs(img.g);
    let first_bucket = 1 + dirs;
    let mut meta = Page::zeroed();
    {
        let m = HashMetaPage::init(&mut meta, img.bucket_cap, 1);
        m.global_depth = img.g;
        m.dir_count = dirs;
        for i in 0..dirs {
            m.dir_pages[i as usize] = 1 + i;
        }
        edit(m);
    }
    s.write_page(0, &meta).unwrap();
    let n = 1u64 << img.g;
    for i in 0..dirs {
        let mut p = Page::zeroed();
        let d: &mut DirectoryPage = bytemuck::cast_mut(&mut p);
        for j in 0..DIR_SLOTS as u64 {
            let slot = i * DIR_SLOTS as u64 + j;
            if slot < n {
                d.slots[j as usize] = first_bucket + (img.slot_bucket)(slot) as u64;
            }
        }
        s.write_page(1 + i, &p).unwrap();
    }
    for (bi, (depth, keys)) in img.buckets.iter().enumerate() {
        let mut p = Page::zeroed();
        let b = BucketPage::init(&mut p, *depth);
        for (i, &k) in keys.iter().enumerate() {
            b.keys[i] = k;
            b.values[i] = !k;
        }
        b.count = keys.len() as u64;
        s.write_page(first_bucket + bi as u64, &p).unwrap();
    }
    s
}

/// Global depth `g`, every slot pointing at one empty bucket of local depth 0.
fn single_bucket_image(g: u64, bucket_cap: u64) -> Image {
    Image {
        g,
        bucket_cap,
        buckets: vec![(0, Vec::new())],
        slot_bucket: |_| 0,
    }
}

fn open_err(s: MemStorage) -> PoolError {
    match PagedHash::open(BufferPool::new(Pager::new(s).unwrap(), 2)) {
        Ok(_) => panic!("open unexpectedly succeeded"),
        Err(e) => e,
    }
}

fn open_ok(s: MemStorage, frames: usize) -> PagedHash<MemStorage> {
    match PagedHash::open(BufferPool::new(Pager::new(s).unwrap(), frames)) {
        Ok(h) => h,
        Err(e) => panic!("open of a valid image failed: {e:?}"),
    }
}

// ---------------------------------------------------------------- helper self-tests

#[test]
fn unmix_inverts_mix() {
    let mut seed = 1;
    for h in [0u64, 1, 1 << 12, u64::MAX, 0x8000_0000_0000_0000] {
        assert_eq!(mix(unmix(h)), h);
    }
    for _ in 0..1000 {
        let h = splitmix(&mut seed);
        assert_eq!(mix(unmix(h)), h);
        assert_eq!(unmix(mix(h)), h);
    }
}

// ---------------------------------------------------------------- create / edge cases

#[test]
fn create_lays_out_meta_directory_and_empty_bucket() {
    let h = mem_hash(4, 2);
    assert_eq!(h.global_depth(), 0);
    assert_eq!(h.bucket_cap(), 4);
    assert_eq!(h.bucket_count().unwrap(), 1);
    assert_eq!(h.pool().pinned_frames(), 0);

    let meta_p = read_page(h.pool(), 0);
    let meta: &HashMetaPage = bytemuck::cast_ref(&meta_p);
    assert_eq!(meta.magic, HASH_MAGIC);
    assert_eq!(
        (meta.global_depth, meta.bucket_cap, meta.dir_count),
        (0, 4, 1)
    );
    assert_eq!(meta.dir_pages(), &[1]);
    assert!(
        meta.dir_pages[1..].iter().all(|&p| p == 0),
        "unused dir_pages zero"
    );

    let dir_p = read_page(h.pool(), 1);
    let dir: &DirectoryPage = bytemuck::cast_ref(&dir_p);
    assert_eq!(dir.slots[0], 2, "slot 0 -> bucket page 2");

    let b_p = read_page(h.pool(), 2);
    let b: &BucketPage = bytemuck::cast_ref(&b_p);
    assert_eq!((b.local_depth, b.count), (0, 0));

    let l = verify(&h);
    assert_eq!(l.slots, vec![2]);
    assert_eq!(l.buckets.len(), 1);
}

#[test]
fn create_and_get_work_with_a_one_frame_pool() {
    let h = mem_hash(3, 1);
    assert_eq!(h.get(0).unwrap(), None);
    assert_eq!(h.get(u64::MAX).unwrap(), None);
    assert_eq!(h.range(0, u64::MAX).unwrap(), vec![]);
    assert_eq!(h.range_rev(0, u64::MAX).unwrap(), vec![]);
    assert_eq!(h.bucket_count().unwrap(), 1);
    assert_eq!(h.pool().pinned_frames(), 0);
    h.check();
}

#[test]
fn reads_work_with_a_one_frame_pool_on_a_populated_file() {
    // get / range hold one pin at a time, so a reopened multi-page hash is readable with 1 frame.
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("h1.db");
    let mut m = BTreeMap::new();
    {
        let mut h = PagedHash::create(file_pool(&path, 2), 2).unwrap();
        for k in 0..300u64 {
            m.insert(k * 7, k);
            h.insert(k * 7, k).unwrap();
        }
        assert!(h.global_depth() >= 5);
        h.pool().flush().unwrap();
    }
    let h = PagedHash::open(file_pool(&path, 1)).unwrap();
    for k in 0..2200u64 {
        assert_eq!(h.get(k).unwrap(), m.get(&k).copied(), "get({k})");
    }
    assert_eq!(h.range(100, 900).unwrap(), model_range(&m, 100, 900));
    assert_eq!(
        h.range_rev(100, 900).unwrap(),
        model_range_rev(&m, 100, 900)
    );
    assert_eq!(h.pool().pinned_frames(), 0);
}

#[test]
fn empty_hash_get_and_ranges() {
    let h = mem_hash(2, 2);
    assert_eq!(h.get(5).unwrap(), None);
    assert_eq!(h.range(5, 5).unwrap(), vec![]);
    assert_eq!(h.range(9, 3).unwrap(), vec![]);
    assert_eq!(h.range_rev(9, 3).unwrap(), vec![]);
    assert_eq!(h.pool().pinned_frames(), 0);
}

#[test]
fn create_panics_for_bucket_cap_0() {
    let r = catch_unwind(|| {
        let _ = PagedHash::create(mem_pool(2), 0);
    });
    let msg = panic_message(r);
    assert!(msg.contains("bucket_cap"), "panic message: {msg:?}");
}

#[test]
fn create_panics_for_bucket_cap_at_page_capacity() {
    for cap in [BUCKET_CAPACITY, BUCKET_CAPACITY + 1, usize::MAX] {
        let r = catch_unwind(|| {
            let _ = PagedHash::create(mem_pool(2), cap);
        });
        let msg = panic_message(r);
        assert!(
            msg.contains("bucket_cap + 1"),
            "cap {cap}: panic message {msg:?}"
        );
    }
}

#[test]
fn create_accepts_bounds_1_and_1022() {
    for cap in [1, BUCKET_CAPACITY - 1] {
        let mut h = mem_hash(cap, 2);
        verify(&h);
        let mut m = BTreeMap::new();
        for k in 0..(cap as u64 + 5) {
            assert_eq!(h.insert(k, k + 1).unwrap(), m.insert(k, k + 1));
        }
        verify(&h);
        assert_matches_model(&h, &m);
    }
}

#[test]
fn full_page_bucket_cap_1022_fills_then_splits() {
    let cap = BUCKET_CAPACITY - 1;
    let mut h = mem_hash(cap, 2);
    let mut m = BTreeMap::new();
    let mut seed = 11;
    while m.len() < cap {
        let k = splitmix(&mut seed);
        assert_eq!(h.insert(k, !k).unwrap(), m.insert(k, !k));
    }
    assert_eq!(h.global_depth(), 0, "no split before bucket_cap + 1 keys");
    assert_eq!(h.bucket_count().unwrap(), 1);
    verify(&h);
    let k = splitmix(&mut seed);
    h.insert(k, 1).unwrap();
    m.insert(k, 1);
    assert!(h.global_depth() >= 1);
    assert!(h.bucket_count().unwrap() >= 2);
    verify(&h);
    assert_matches_model(&h, &m);
}

#[test]
fn first_split_happens_exactly_at_bucket_cap_plus_one() {
    for cap in [1usize, 2, 3, 4, 7] {
        let mut h = mem_hash(cap, 2);
        // Keys alternate hash bit 0 so one split (depth 1) is enough to separate them.
        let keys: Vec<u64> = (0..=cap as u64).map(|i| unmix(i << 8 | (i & 1))).collect();
        for &k in &keys[..cap] {
            h.insert(k, 1).unwrap();
            assert_eq!(h.global_depth(), 0, "cap {cap}: split too early");
            assert_eq!(h.bucket_count().unwrap(), 1);
        }
        h.insert(keys[cap], 1).unwrap();
        assert_eq!(h.global_depth(), 1, "cap {cap}: one split at cap + 1");
        assert_eq!(h.bucket_count().unwrap(), 2);
        let l = verify(&h);
        for b in l.buckets.values() {
            assert_eq!(b.local_depth, 1);
        }
    }
}

#[test]
fn upsert_returns_previous_value() {
    let mut h = mem_hash(2, 2);
    assert_eq!(h.insert(7, 70).unwrap(), None);
    assert_eq!(h.insert(7, 71).unwrap(), Some(70));
    assert_eq!(h.insert(7, 72).unwrap(), Some(71));
    assert_eq!(h.get(7).unwrap(), Some(72));
    for k in 0..80 {
        h.insert(k * 10, k).unwrap();
    }
    let before = verify(&h);
    for k in 0..80 {
        assert_eq!(
            h.insert(k * 10, k + 1000).unwrap(),
            Some(k),
            "upsert {}",
            k * 10
        );
    }
    let after = verify(&h);
    assert_eq!(after.pairs().len(), 81, "upsert must not add keys");
    assert_eq!(
        (after.global_depth, after.slots.clone()),
        (before.global_depth, before.slots.clone()),
        "upsert must not split"
    );
    for k in 0..80 {
        assert_eq!(h.get(k * 10).unwrap(), Some(k + 1000));
    }
    assert_eq!(h.get(7).unwrap(), Some(72));
}

#[test]
fn extreme_keys_zero_and_max() {
    let mut h = mem_hash(1, 2);
    let mut m = BTreeMap::new();
    for k in [u64::MAX, 0, u64::MAX - 1, 1, u64::MAX / 2, 2, u64::MAX - 2] {
        assert_eq!(h.insert(k, !k).unwrap(), m.insert(k, !k));
        verify(&h);
    }
    assert_matches_model(&h, &m);
    assert_eq!(h.get(u64::MAX).unwrap(), Some(0));
    assert_eq!(h.get(0).unwrap(), Some(u64::MAX));
    assert_eq!(h.range(u64::MAX, u64::MAX).unwrap(), vec![(u64::MAX, 0)]);
    assert_eq!(h.range(0, 0).unwrap(), vec![(0, u64::MAX)]);
    assert_eq!(
        h.range_rev(u64::MAX - 1, u64::MAX).unwrap(),
        vec![(u64::MAX, 0), (u64::MAX - 1, 1)]
    );
    assert_eq!(h.insert(u64::MAX, 5).unwrap(), Some(0));
}

#[test]
fn range_lo_gt_hi_is_empty_and_lo_eq_hi_is_point() {
    let mut h = mem_hash(3, 2);
    for k in 0..100 {
        h.insert(k * 2, k).unwrap();
    }
    assert_eq!(h.range(10, 9).unwrap(), vec![]);
    assert_eq!(h.range_rev(10, 9).unwrap(), vec![]);
    assert_eq!(h.range(u64::MAX, 0).unwrap(), vec![]);
    assert_eq!(h.range_rev(u64::MAX, 0).unwrap(), vec![]);
    assert_eq!(h.range(10, 10).unwrap(), vec![(10, 5)]);
    assert_eq!(h.range_rev(10, 10).unwrap(), vec![(10, 5)]);
    assert_eq!(h.range(11, 11).unwrap(), vec![]);
    assert_eq!(h.range(197, 1000).unwrap(), vec![(198, 99)]);
    assert_eq!(h.range_rev(0, 1).unwrap(), vec![(0, 0)]);
    assert_eq!(h.range(199, u64::MAX).unwrap(), vec![]);
    assert_eq!(
        h.range_rev(20, 26).unwrap(),
        vec![(26, 13), (24, 12), (22, 11), (20, 10)]
    );
    assert_eq!(h.pool().pinned_frames(), 0);
}

// ---------------------------------------------------------------- directory growth (S3)

#[test]
fn shared_low_12_bits_force_a_multi_page_directory() {
    // Two keys whose hashes agree on the low 12 bits and differ at bit 12: with bucket_cap 1
    // the directory must grow to exactly 2^13 slots = 4 directory pages.
    let mut h = mem_hash(1, 2);
    let a = unmix(0x5A5);
    let b = unmix(0x5A5 | 1 << 12);
    h.insert(a, 1).unwrap();
    h.insert(b, 2).unwrap();
    assert_eq!(h.global_depth(), 13);
    assert_eq!(
        h.bucket_count().unwrap(),
        14,
        "one bucket per split level + the pair"
    );
    let l = verify(&h);
    assert_eq!(l.dir_pages.len(), 4);
    let meta_p = read_page(h.pool(), 0);
    let meta: &HashMetaPage = bytemuck::cast_ref(&meta_p);
    assert_eq!((meta.global_depth, meta.dir_count), (13, 4));
    assert_ne!(l.bucket_of(a), l.bucket_of(b));
    assert_eq!(h.get(a).unwrap(), Some(1));
    assert_eq!(h.get(b).unwrap(), Some(2));
    // A key landing in the last directory page (slot 2^13 - 1).
    let c = unmix((1 << 13) - 1);
    h.insert(c, 3).unwrap();
    assert_eq!(h.get(c).unwrap(), Some(3));
    verify(&h);
}

#[test]
fn two_frame_pool_many_inserts_grow_directory_past_one_page() {
    let mut h = mem_hash(2, 2);
    let mut m = BTreeMap::new();
    let mut seed = 99;
    let mut i = 0u64;
    while h.global_depth() < 12 {
        let k = if i.is_multiple_of(3) {
            i
        } else {
            splitmix(&mut seed)
        };
        assert_eq!(h.insert(k, i).unwrap(), m.insert(k, i));
        assert_eq!(h.pool().pinned_frames(), 0, "insert leaked a pin");
        let _ = h.get(k).unwrap();
        assert_eq!(h.pool().pinned_frames(), 0, "get leaked a pin");
        if i.is_multiple_of(64) {
            h.check();
        }
        i += 1;
        assert!(i < 20_000, "directory never passed 2048 slots");
    }
    let l = verify(&h);
    assert!(l.dir_pages.len() >= 2);
    h.pool().flush().unwrap();
    let meta_p = read_page(h.pool(), 0);
    let meta: &HashMetaPage = bytemuck::cast_ref(&meta_p);
    assert!(meta.dir_count >= 2, "meta dir_count {}", meta.dir_count);
    assert_eq!(meta.dir_count, ceil_dirs(h.global_depth() as u64));
    assert_matches_model(&h, &m);
    assert_eq!(h.range(1000, 5000).unwrap(), model_range(&m, 1000, 5000));
    assert_eq!(
        h.range_rev(1000, 5000).unwrap(),
        model_range_rev(&m, 1000, 5000)
    );
    assert_eq!(h.pool().pinned_frames(), 0);
    assert!(
        h.pool().stats().evictions > 0,
        "2-frame pool must have evicted"
    );
}

#[test]
fn meta_page_tracks_global_depth_after_inserts() {
    let mut h = mem_hash(1, 3);
    let mut last = 0;
    let mut seed = 7;
    for _ in 0..600 {
        let k = splitmix(&mut seed) % 5000;
        h.insert(k, k).unwrap();
        assert!(h.global_depth() >= last, "global depth decreased");
        last = h.global_depth();
        h.pool().flush().unwrap();
        assert_eq!(h.pool().pinned_frames(), 0);
        let meta_p = read_page(h.pool(), 0);
        let meta: &HashMetaPage = bytemuck::cast_ref(&meta_p);
        assert_eq!(meta.global_depth, h.global_depth() as u64);
        assert_eq!(meta.dir_count, ceil_dirs(meta.global_depth));
        assert_eq!(meta.bucket_cap, 1);
    }
    assert!(last >= 8, "600 keys at cap 1 must split deep (got {last})");
    verify(&h);
}

#[test]
fn depth_21_image_opens_and_splitting_beyond_21_is_an_error() {
    // Comb image at global depth 21 (1024 directory pages): bucket j (j = 0..21) has local
    // depth j+1 and owns the slots whose lowest set bit is j; the last bucket owns slot 0 at
    // local depth 21. Every bucket is valid, so open + check must accept it.
    let zero_low21 = |i: u64| unmix(i << 21);
    let img = Image {
        g: 21,
        bucket_cap: 1,
        buckets: (0..21)
            .map(|j| (j + 1, Vec::new()))
            .chain(std::iter::once((21, vec![zero_low21(1)])))
            .collect(),
        slot_bucket: |s| {
            if s == 0 {
                21
            } else {
                s.trailing_zeros() as usize
            }
        },
    };
    let s = build_storage(&img, |_| {});
    let mut h = open_ok(s, 2);
    assert_eq!(h.global_depth(), 21);
    assert_eq!(h.bucket_count().unwrap(), 22);
    let l = verify(&h);
    assert_eq!(l.dir_pages.len(), 1024);
    assert_eq!(h.get(zero_low21(1)).unwrap(), Some(!zero_low21(1)));

    // Splitting a shallow bucket needs no doubling: fine at depth 21.
    let odd_a = unmix(1);
    let odd_b = unmix(3);
    assert_eq!(h.insert(odd_a, 10).unwrap(), None);
    assert_eq!(h.insert(odd_b, 11).unwrap(), None);
    assert_eq!(h.global_depth(), 21);
    assert_eq!(h.bucket_count().unwrap(), 23);
    assert_eq!(h.pool().pinned_frames(), 0);

    // A second key with low 21 hash bits = 0 needs global depth 22: 2^22 slots do not fit
    // in 2044 directory pages, so insert must return Err (not panic, not corrupt silently).
    const { assert!(MAX_DIR_PAGES * DIR_SLOTS < 1 << 22) };
    let r = h.insert(zero_low21(2), 12);
    assert!(
        r.is_err(),
        "doubling past 2044 * 2048 slots must be Err, got {r:?}"
    );
    assert_eq!(h.global_depth(), 21);
    assert_eq!(h.pool().pinned_frames(), 0, "error path leaked a pin");
}

// ---------------------------------------------------------------- Index trait

#[test]
fn index_trait_object_matches_model() {
    let mut h = mem_hash(3, 3);
    let mut m = BTreeMap::new();
    {
        let idx: &mut dyn Index = &mut h;
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
        assert_eq!(idx.range_rev(150, 50), vec![]);
    }
    verify(&h);
    assert_matches_model(&h, &m);
}

// ---------------------------------------------------------------- persistence (P5)

#[test]
fn persistence_reopen_preserves_everything_and_keeps_working() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("h.db");
    let cap = 3;
    let mut m = BTreeMap::new();
    let mut seed = 2026;
    let before = {
        let mut h = PagedHash::create(file_pool(&path, 3), cap).unwrap();
        for i in 0..3000u64 {
            let k = splitmix(&mut seed) % 20_000;
            assert_eq!(h.insert(k, i).unwrap(), m.insert(k, i));
        }
        let l = verify(&h);
        h.pool().flush().unwrap();
        (h.global_depth(), h.bucket_count().unwrap(), l)
    }; // dropped: file closed

    let mut h = PagedHash::open(file_pool(&path, 2)).unwrap();
    assert_eq!(h.bucket_cap(), cap, "bucket_cap from meta");
    assert_eq!(
        (h.global_depth(), h.bucket_count().unwrap()),
        (before.0, before.1),
        "global_depth / bucket_count after reopen"
    );
    assert_eq!(verify(&h), before.2, "page structure changed across reopen");
    assert_matches_model(&h, &m);

    // Keep inserting after reopen: new pages must not overwrite old ones.
    for i in 0..3000u64 {
        let k = splitmix(&mut seed) % 40_000;
        assert_eq!(
            h.insert(k, i + 1_000_000).unwrap(),
            m.insert(k, i + 1_000_000)
        );
    }
    verify(&h);
    assert_matches_model(&h, &m);

    // Second round trip.
    h.pool().flush().unwrap();
    let (g, bc) = (h.global_depth(), h.bucket_count().unwrap());
    drop(h);
    let h = PagedHash::open(file_pool(&path, 4)).unwrap();
    assert_eq!((h.global_depth(), h.bucket_count().unwrap()), (g, bc));
    verify(&h);
    assert_matches_model(&h, &m);
}

#[test]
fn persistence_of_empty_and_unsplit_hashes() {
    let dir = tempfile::tempdir().unwrap();
    for (n, name) in [(0u64, "empty.db"), (3, "one.db")] {
        let path = dir.path().join(name);
        {
            let mut h = PagedHash::create(file_pool(&path, 2), 5).unwrap();
            for k in 0..n {
                h.insert(k, k + 1).unwrap();
            }
            h.pool().flush().unwrap();
        }
        let h = PagedHash::open(file_pool(&path, 2)).unwrap();
        assert_eq!((h.global_depth(), h.bucket_count().unwrap()), (0, 1));
        assert_eq!(h.bucket_cap(), 5);
        let l = verify(&h);
        assert_eq!(l.pairs(), (0..n).map(|k| (k, k + 1)).collect::<Vec<_>>());
    }
}

// ---------------------------------------------------------------- open() rejects

#[test]
fn open_accepts_hand_built_valid_images() {
    // Positive controls for the rejection tests below: same builder, untouched fields.
    for (g, cap) in [(0u64, 1u64), (0, 1022), (11, 4), (12, 4)] {
        let mut h = open_ok(build_storage(&single_bucket_image(g, cap), |_| {}), 2);
        assert_eq!(h.global_depth() as u64, g);
        assert_eq!(h.bucket_cap() as u64, cap);
        assert_eq!(h.bucket_count().unwrap(), 1);
        verify(&h);
        for k in 0..20 {
            h.insert(k, k).unwrap();
        }
        verify(&h);
    }
}

#[test]
fn open_rejects_zeroed_page_0() {
    let s = build_storage(&single_bucket_image(0, 4), |m| *m = HashMetaPage::zeroed());
    assert_eq!(io_kind(open_err(s)), io::ErrorKind::InvalidData);
}

#[test]
fn open_rejects_wrong_magic_only() {
    for magic in [BTREE_MAGIC, HASH_MAGIC ^ 1, 0] {
        let s = build_storage(&single_bucket_image(0, 4), |m| m.magic = magic);
        assert_eq!(
            io_kind(open_err(s)),
            io::ErrorKind::InvalidData,
            "magic {magic:#x}"
        );
    }
}

#[test]
fn open_rejects_a_real_btree_file() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("bt.db");
    {
        let mut t = PagedBTree::create(file_pool(&path, 2), 4).unwrap();
        for k in 0..50 {
            t.insert(k, k).unwrap();
        }
        t.pool().flush().unwrap();
    }
    match PagedHash::open(file_pool(&path, 2)) {
        Ok(_) => panic!("opened a B+tree file as a hash"),
        Err(e) => assert_eq!(io_kind(e), io::ErrorKind::InvalidData),
    }
}

#[test]
fn open_rejects_bucket_cap_out_of_range() {
    for cap in [
        0u64,
        BUCKET_CAPACITY as u64,
        BUCKET_CAPACITY as u64 + 1,
        u64::MAX,
    ] {
        let s = build_storage(&single_bucket_image(0, 4), |m| m.bucket_cap = cap);
        assert_eq!(
            io_kind(open_err(s)),
            io::ErrorKind::InvalidData,
            "bucket_cap {cap}"
        );
    }
}

#[test]
fn open_rejects_global_depth_above_21() {
    // No dir_count is consistent with depth >= 22 (2^22 / 2048 = 2048 > MAX_DIR_PAGES), so the
    // depth check and the dir_count check necessarily overlap here; dir_count is set to the
    // largest storable value so that only an out-of-range read could slip through.
    for g in [22u64, 23, 63, 64, u64::MAX] {
        let s = build_storage(&single_bucket_image(0, 4), |m| {
            m.global_depth = g;
            m.dir_count = MAX_DIR_PAGES as u64;
        });
        assert_eq!(
            io_kind(open_err(s)),
            io::ErrorKind::InvalidData,
            "global_depth {g}"
        );
    }
}

#[test]
fn open_rejects_dir_count_mismatch() {
    // Depth 12 needs exactly 2 directory pages (valid in open_accepts_hand_built_valid_images).
    for dc in [
        0u64,
        1,
        3,
        MAX_DIR_PAGES as u64,
        MAX_DIR_PAGES as u64 + 1,
        u64::MAX,
    ] {
        let s = build_storage(&single_bucket_image(12, 4), |m| {
            if dc == 3 {
                m.dir_pages[2] = 2; // a real directory page, so only the count is wrong
            }
            m.dir_count = dc;
        });
        assert_eq!(
            io_kind(open_err(s)),
            io::ErrorKind::InvalidData,
            "dir_count {dc}"
        );
    }
    // Depth 0 needs exactly 1.
    for dc in [0u64, 2] {
        let s = build_storage(&single_bucket_image(0, 4), |m| {
            m.dir_pages[1] = 1;
            m.dir_count = dc;
        });
        assert_eq!(
            io_kind(open_err(s)),
            io::ErrorKind::InvalidData,
            "depth 0, dir_count {dc}"
        );
    }
}

// ---------------------------------------------------------------- model proptests (I1/I3/I4)

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

/// Run `ops` against hash + model, continuing from `m`; `check()` after every insert.
fn run_model<S: Storage>(h: &mut PagedHash<S>, m: &mut BTreeMap<u64, u64>, ops: &[Op]) {
    for o in ops {
        match *o {
            Op::Insert(k, v) => {
                assert_eq!(h.insert(k, v).unwrap(), m.insert(k, v), "insert({k},{v})");
                h.check();
            }
            Op::Get(k) => assert_eq!(h.get(k).unwrap(), m.get(&k).copied(), "get({k})"),
            Op::Range(a, b) => {
                assert_eq!(
                    h.range(a, b).unwrap(),
                    model_range(m, a, b),
                    "range({a},{b})"
                )
            }
            Op::RangeRev(a, b) => assert_eq!(
                h.range_rev(a, b).unwrap(),
                model_range_rev(m, a, b),
                "range_rev({a},{b})"
            ),
        }
        assert_eq!(h.pool().pinned_frames(), 0, "pin leak after {o:?}");
    }
    verify(h);
    assert_matches_model(h, m);
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(256))]

    #[test]
    fn mem_hash_matches_btreemap(
        cap in 1usize..=4,
        frames in 2usize..=4,
        ops in prop::collection::vec(op(), 0..200),
    ) {
        let mut h = mem_hash(cap, frames);
        let mut m = BTreeMap::new();
        run_model(&mut h, &mut m, &ops);
    }
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(24))]

    #[test]
    fn file_hash_matches_btreemap_and_survives_reopen(
        cap in 1usize..=4,
        frames in 2usize..=4,
        ops in prop::collection::vec(op(), 0..200),
        more in prop::collection::vec(op(), 0..100),
    ) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("p.db");
        let mut h = PagedHash::create(file_pool(&path, frames), cap).unwrap();
        let mut m = BTreeMap::new();
        run_model(&mut h, &mut m, &ops);
        h.pool().flush().unwrap();
        let (g, bc) = (h.global_depth(), h.bucket_count().unwrap());
        let l = layout(h.pool());
        drop(h);

        let mut h = PagedHash::open(file_pool(&path, frames)).unwrap();
        prop_assert_eq!((h.global_depth(), h.bucket_count().unwrap()), (g, bc));
        prop_assert_eq!(h.bucket_cap(), cap);
        prop_assert_eq!(verify(&h), l);
        assert_matches_model(&h, &m);
        run_model(&mut h, &mut m, &more);
    }
}

// ---------------------------------------------------------------- check() is not fail-open

/// Build a valid hash (bucket_cap 2, 60 keys), confirm it passes, apply `corrupt`, expect
/// `PagedHash::check` to panic.
fn check_rejects(corrupt: impl Fn(&BufferPool<MemStorage>, &Layout)) {
    let mut h = mem_hash(2, 4);
    for k in 0..60u64 {
        h.insert(k * 3, k).unwrap();
    }
    let l = verify(&h);
    assert!(l.global_depth >= 3, "need a few levels of splits");
    corrupt(h.pool(), &l);
    let r = catch_unwind(AssertUnwindSafe(|| h.check()));
    assert!(r.is_err(), "check() accepted a corrupted hash");
}

fn with_bucket(pool: &BufferPool<MemStorage>, id: PageId, f: impl FnOnce(&mut BucketPage)) {
    let mut g = pool.fetch_mut(id).unwrap();
    f(bytemuck::cast_mut(&mut *g));
}

fn set_slot(pool: &BufferPool<MemStorage>, l: &Layout, slot: u64, to: PageId) {
    let dp = l.dir_pages[slot as usize / DIR_SLOTS];
    let mut g = pool.fetch_mut(dp).unwrap();
    let d: &mut DirectoryPage = bytemuck::cast_mut(&mut *g);
    d.slots[slot as usize % DIR_SLOTS] = to;
}

/// Two distinct non-empty buckets.
fn two_nonempty(l: &Layout) -> (PageId, PageId) {
    let ids: Vec<PageId> = l
        .buckets
        .iter()
        .filter(|(_, b)| !b.keys.is_empty())
        .map(|(&id, _)| id)
        .collect();
    (ids[0], ids[1])
}

#[test]
fn check_detects_key_in_wrong_bucket() {
    // Swap one key between two buckets: counts and key set unchanged, only suffixes break.
    check_rejects(|pool, l| {
        let (a, b) = two_nonempty(l);
        let ka = l.buckets[&a].keys[0];
        let kb = l.buckets[&b].keys[0];
        with_bucket(pool, a, |p| p.keys[0] = kb);
        with_bucket(pool, b, |p| p.keys[0] = ka);
    });
}

#[test]
fn check_detects_bucket_over_cap() {
    // Append fresh keys with the right suffix so only count <= bucket_cap can fire.
    check_rejects(|pool, l| {
        let (&id, b) = l.buckets.iter().next().unwrap();
        let suffix = low(b.slots[0], b.local_depth);
        let mut n = b.keys.len();
        let mut i = 1u64;
        with_bucket(pool, id, |p| {
            while n <= l.bucket_cap as usize {
                let k = unmix(suffix | (i << b.local_depth.max(1)) << 20);
                i += 1;
                assert_eq!(low(mix(k), b.local_depth), suffix);
                p.keys[n] = k;
                p.values[n] = 0;
                n += 1;
            }
            p.count = n as u64;
        });
    });
}

#[test]
fn check_detects_local_depth_above_global() {
    check_rejects(|pool, l| {
        let (&id, _) = l.buckets.iter().next().unwrap();
        with_bucket(pool, id, |p| p.local_depth = l.global_depth + 1);
    });
}

#[test]
fn check_detects_slot_redirected_to_another_bucket() {
    check_rejects(|pool, l| {
        // A slot whose bucket is shared by >= 2 slots if any, else any slot.
        let shared = l
            .buckets
            .values()
            .find(|b| b.slots.len() >= 2)
            .map(|b| b.slots[1])
            .unwrap_or(0);
        let from = l.slots[shared as usize];
        let to = *l.buckets.keys().find(|&&id| id != from).unwrap();
        set_slot(pool, l, shared, to);
    });
}

#[test]
fn check_detects_slot_pointing_at_meta_or_directory() {
    check_rejects(|pool, l| set_slot(pool, l, 1, 0));
    check_rejects(|pool, l| set_slot(pool, l, 1, l.dir_pages[0]));
}

#[test]
fn check_detects_changed_meta_global_depth() {
    check_rejects(|pool, _| {
        let mut g = pool.fetch_mut(0).unwrap();
        let m: &mut HashMetaPage = bytemuck::cast_mut(&mut *g);
        m.global_depth += 1;
    });
    check_rejects(|pool, _| {
        let mut g = pool.fetch_mut(0).unwrap();
        let m: &mut HashMetaPage = bytemuck::cast_mut(&mut *g);
        m.global_depth -= 1;
    });
}

#[test]
fn check_detects_changed_meta_bucket_cap_and_magic() {
    check_rejects(|pool, _| {
        let mut g = pool.fetch_mut(0).unwrap();
        let m: &mut HashMetaPage = bytemuck::cast_mut(&mut *g);
        m.bucket_cap += 1;
    });
    check_rejects(|pool, _| {
        let mut g = pool.fetch_mut(0).unwrap();
        let m: &mut HashMetaPage = bytemuck::cast_mut(&mut *g);
        m.magic = BTREE_MAGIC;
    });
}

#[test]
fn check_detects_duplicated_key_across_buckets() {
    // Copy a key into another bucket that has room (also a suffix violation there).
    check_rejects(|pool, l| {
        let (a, _) = two_nonempty(l);
        let k = l.buckets[&a].keys[0];
        let (&b, bb) = l
            .buckets
            .iter()
            .find(|(id, b)| **id != a && b.keys.len() < l.bucket_cap as usize)
            .expect("a bucket with room");
        let n = bb.keys.len();
        with_bucket(pool, b, |p| {
            p.keys[n] = k;
            p.values[n] = 0;
            p.count += 1;
        });
    });
}

#[test]
fn check_detects_duplicated_key_within_a_bucket() {
    // Same key twice in one bucket that has room: suffix and count stay valid, only the
    // no-duplicates rule can fire.
    check_rejects(|pool, l| {
        let (&id, b) = l
            .buckets
            .iter()
            .find(|(_, b)| b.keys.len() == 1)
            .expect("a bucket holding exactly one key");
        let k = b.keys[0];
        with_bucket(pool, id, |p| {
            p.keys[1] = k;
            p.values[1] = 0;
            p.count = 2;
        });
    });
}

#[test]
fn check_rejects_helper_needs_a_corruption() {
    // Sanity: without corruption, check_rejects' own assertion must fail.
    let r = catch_unwind(|| check_rejects(|_, _| {}));
    assert!(
        r.is_err(),
        "check_rejects passed without corrupting anything"
    );
}

#[test]
fn layout_keys_are_unique_and_slots_cover_every_key() {
    let mut h = mem_hash(2, 2);
    let mut seed = 3;
    let mut keys = BTreeSet::new();
    for _ in 0..500 {
        let k = splitmix(&mut seed);
        keys.insert(k);
        h.insert(k, 0).unwrap();
    }
    let l = verify(&h);
    for &k in &keys {
        let b = l.bucket_of(k);
        assert!(
            l.buckets[&b].keys.contains(&k),
            "key {k} not in its directory bucket"
        );
    }
    assert_eq!(l.pairs().len(), keys.len());
}

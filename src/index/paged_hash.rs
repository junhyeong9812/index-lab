//! 페이지 Extendible Hash — 1단계 `hash.rs`의 알고리즘을 버퍼풀 위 16KB 페이지로 옮긴 것.
//!
//! 조회 경로: 메타(메모리 사본) → 디렉터리 페이지 1장 → 버킷 페이지 1장.
//! 디렉터리 칸에서 버킷 번호만 꺼내고 핀을 놓은 뒤 버킷을 잡는다 → 한 번에 핀 1개.

use crate::storage::pool::{BufferPool, Result};
use crate::storage::{PageId, Storage};

use super::hash::mix;
use super::hash_page::{BUCKET_CAPACITY, BucketPage, DIR_SLOTS, DirectoryPage, HashMetaPage};

const META: PageId = 0;

pub struct PagedHash<S: Storage> {
    pool: BufferPool<S>,
    /// 메타 페이지의 사본 — 바뀔 때마다 페이지 0에도 쓴다(diff 10).
    global_depth: u32,
    bucket_cap: usize,
    dir_pages: Vec<PageId>,
}

impl<S: Storage> PagedHash<S> {
    /// 빈 저장소 위에 새 해시를 만든다: 페이지 0 = 메타, 1 = 디렉터리(1칸), 2 = 빈 버킷(지역 깊이 0).
    ///
    /// 삽입은 "넣고 넘치면 나눈다"라 버킷이 잠깐 `bucket_cap + 1`개를 담는다 → 그 한 칸까지 페이지에 들어가야 한다.
    pub fn create(pool: BufferPool<S>, bucket_cap: usize) -> Result<Self> {
        assert!(bucket_cap >= 1, "bucket_cap은 1 이상이어야 한다");
        assert!(
            bucket_cap < BUCKET_CAPACITY,
            "bucket_cap + 1이 버킷 페이지 용량({BUCKET_CAPACITY})을 넘는다"
        );
        // 한 번에 한 장씩 만든다(핀 1개) — 프레임 1개 풀에서도 만들 수 있게.
        let meta_id = pool.new_page()?.0;
        assert_eq!(
            meta_id, META,
            "create는 빈 저장소에서만 — 첫 페이지가 메타여야 한다"
        );
        let dir_id = pool.new_page()?.0;
        let bucket_id = {
            let (id, mut bucket) = pool.new_page()?;
            BucketPage::init(&mut bucket, 0);
            id
        };
        {
            let mut dir = pool.fetch_mut(dir_id)?;
            let d: &mut DirectoryPage = bytemuck::cast_mut(&mut *dir);
            d.slots[0] = bucket_id; // 전역 깊이 0 → 칸 1개
        }
        HashMetaPage::init(&mut *pool.fetch_mut(META)?, bucket_cap as u64, dir_id);
        Ok(Self {
            pool,
            global_depth: 0,
            bucket_cap,
            dir_pages: vec![dir_id],
        })
    }

    pub fn get(&self, key: u64) -> Result<Option<u64>> {
        let bucket_id = self.bucket_of(key)?;
        let page = self.pool.fetch(bucket_id)?;
        let b: &BucketPage = bytemuck::cast_ref(&*page);
        Ok(b.keys()
            .iter()
            .position(|&k| k == key)
            .map(|i| b.values()[i]))
    }

    /// 키가 갈 버킷 페이지 번호 — 디렉터리 페이지를 잡아 칸 하나만 읽고 바로 놓는다.
    fn bucket_of(&self, key: u64) -> Result<PageId> {
        let slot = self.slot(key);
        let page = self.pool.fetch(self.dir_pages[slot / DIR_SLOTS])?;
        let dir: &DirectoryPage = bytemuck::cast_ref(&*page);
        Ok(dir.slots[slot % DIR_SLOTS])
    }

    /// 해시의 하위 `global_depth` 비트 = 디렉터리 칸 번호(1단계 `Hash::slot`과 같다).
    fn slot(&self, key: u64) -> usize {
        let mask = (1u64 << self.global_depth) - 1;
        (mix(key) & mask) as usize
    }

    pub fn global_depth(&self) -> u32 {
        self.global_depth
    }

    pub fn bucket_cap(&self) -> usize {
        self.bucket_cap
    }

    pub fn pool(&self) -> &BufferPool<S> {
        &self.pool
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::MemStorage;
    use crate::storage::pager::Pager;

    fn hash(frames: usize, bucket_cap: usize) -> PagedHash<MemStorage> {
        let pool = BufferPool::new(Pager::new(MemStorage::new()).unwrap(), frames);
        PagedHash::create(pool, bucket_cap).unwrap()
    }

    /// 버킷 페이지에 (키, 값)을 직접 써 넣는다 — insert(diff 10) 없이 get만 시험하려고.
    fn put(t: &PagedHash<MemStorage>, bucket: PageId, key: u64, value: u64) {
        let mut page = t.pool().fetch_mut(bucket).unwrap();
        let b: &mut BucketPage = bytemuck::cast_mut(&mut *page);
        let n = b.count as usize;
        b.keys[n] = key;
        b.values[n] = value;
        b.count += 1;
    }

    fn set_slot(t: &PagedHash<MemStorage>, dir: PageId, i: usize, bucket: PageId) {
        let mut page = t.pool().fetch_mut(dir).unwrap();
        let d: &mut DirectoryPage = bytemuck::cast_mut(&mut *page);
        d.slots[i] = bucket;
    }

    #[test]
    fn create_writes_meta_directory_and_empty_bucket() {
        let t = hash(4, 3);
        t.pool().flush().unwrap();
        let page = t.pool().fetch(META).unwrap();
        let m: &HashMetaPage = bytemuck::cast_ref(&*page);
        assert_eq!(
            (m.magic, m.global_depth, m.bucket_cap, m.dir_pages()),
            (super::super::hash_page::HASH_MAGIC, 0, 3, &[1][..])
        );
        drop(page);
        let page = t.pool().fetch(1).unwrap();
        let d: &DirectoryPage = bytemuck::cast_ref(&*page);
        assert_eq!(d.slots[0], 2);
        drop(page);
        let page = t.pool().fetch(2).unwrap();
        let b: &BucketPage = bytemuck::cast_ref(&*page);
        assert_eq!((b.local_depth, b.keys()), (0, &[][..]));
        drop(page);
        assert_eq!(t.pool().pinned_frames(), 0);
    }

    #[test]
    #[should_panic(expected = "bucket_cap")]
    fn create_rejects_bucket_cap_0() {
        hash(4, 0);
    }

    #[test]
    #[should_panic(expected = "bucket_cap + 1")]
    fn create_rejects_bucket_cap_at_page_capacity() {
        hash(4, BUCKET_CAPACITY);
    }

    #[test]
    fn get_on_empty_is_none() {
        let t = hash(4, 3);
        assert_eq!(t.get(42).unwrap(), None);
        assert_eq!(t.pool().pinned_frames(), 0);
    }

    /// 손으로 만든 전역 깊이 1 구조: 칸 0 → 버킷 2, 칸 1 → 새 버킷. 키는 `mix`의 0번 비트로 나눈다.
    #[test]
    fn get_follows_the_low_bit_of_mix_to_the_right_bucket() {
        let mut t = hash(2, 8);
        let b1 = {
            let (id, mut page) = t.pool().new_page().unwrap();
            BucketPage::init(&mut page, 1);
            id
        };
        {
            let mut page = t.pool().fetch_mut(2).unwrap();
            let b: &mut BucketPage = bytemuck::cast_mut(&mut *page);
            b.local_depth = 1;
        }
        set_slot(&t, 1, 1, b1);
        t.global_depth = 1;
        for k in 0..12u64 {
            let bucket = if mix(k) & 1 == 0 { 2 } else { b1 };
            put(&t, bucket, k, k * 10);
        }
        for k in 0..12u64 {
            assert_eq!(t.get(k).unwrap(), Some(k * 10), "key {k}");
        }
        assert_eq!(t.get(99).unwrap(), None);
        assert_eq!(t.pool().pinned_frames(), 0, "핀 누수");
    }

    /// 칸 번호가 2048 이상이면 두 번째 디렉터리 페이지의 `slot - 2048`칸을 읽어야 한다.
    /// (I3을 만족하는 구조가 아니다 — 칸 → 페이지·칸 위치 계산만 시험한다.)
    #[test]
    fn slots_past_2048_live_in_the_second_directory_page() {
        let mut t = hash(2, 8);
        t.global_depth = 12; // 칸 4096개 = 디렉터리 페이지 2장
        let dir1 = t.pool().new_page().unwrap().0;
        t.dir_pages.push(dir1);
        let far = (0..).find(|&k| t.slot(k) >= DIR_SLOTS).unwrap();
        let near = (0..).find(|&k| t.slot(k) < DIR_SLOTS).unwrap();
        let other = {
            let (id, mut page) = t.pool().new_page().unwrap();
            BucketPage::init(&mut page, 12);
            id
        };
        for i in 0..DIR_SLOTS {
            set_slot(&t, 1, i, 2);
            set_slot(&t, dir1, i, 2);
        }
        set_slot(&t, dir1, t.slot(far) - DIR_SLOTS, other);
        put(&t, other, far, 7);
        put(&t, 2, near, 8);
        assert_eq!(t.get(far).unwrap(), Some(7));
        assert_eq!(t.get(near).unwrap(), Some(8));
        assert_eq!(t.pool().pinned_frames(), 0);
    }

    /// 조회 1회 = 디렉터리 1장 + 버킷 1장. 프레임 1개 풀에서도 된다(동시 핀 1).
    #[test]
    fn get_needs_only_one_frame() {
        let t = hash(1, 3);
        put(&t, 2, 5, 50);
        assert_eq!(t.get(5).unwrap(), Some(50));
        let before = t.pool().stats();
        t.get(5).unwrap();
        let after = t.pool().stats();
        assert_eq!(
            after.hits + after.misses - before.hits - before.misses,
            2,
            "fetch 2회"
        );
    }
}

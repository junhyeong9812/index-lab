//! 페이지 Extendible Hash — 1단계 `hash.rs`의 알고리즘을 버퍼풀 위 16KB 페이지로 옮긴 것.
//!
//! 조회 경로: 메타(메모리 사본) → 디렉터리 페이지 1장 → 버킷 페이지 1장.
//! 디렉터리 칸에서 버킷 번호만 꺼내고 핀을 놓은 뒤 버킷을 잡는다 → 한 번에 핀 1개.
//! 삽입의 분할·디렉터리 두 배는 한 번에 핀 최대 2개(옛 페이지 + 새 페이지) — 버퍼풀 최소 프레임 2.

use crate::storage::pool::{BufferPool, PoolError, Result};
use crate::storage::{PageId, Storage};

use super::hash::mix;
use super::hash_page::{
    BUCKET_CAPACITY, BucketPage, DIR_SLOTS, DirectoryPage, HashMetaPage, MAX_DIR_PAGES,
};

const META: PageId = 0;

pub struct PagedHash<S: Storage> {
    pool: BufferPool<S>,
    /// 메타 페이지의 사본 — 두 배가 될 때마다 페이지 0에도 쓴다(`write_meta`).
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

    /// 같은 키가 있으면 값을 바꾸고 이전 값을 돌려준다(upsert).
    pub fn insert(&mut self, key: u64, value: u64) -> Result<Option<u64>> {
        let id = self.bucket_of(key)?;
        {
            let mut page = self.pool.fetch_mut(id)?;
            let b: &mut BucketPage = bytemuck::cast_mut(&mut *page);
            let n = b.count as usize;
            if let Some(i) = b.keys[..n].iter().position(|&k| k == key) {
                return Ok(Some(std::mem::replace(&mut b.values[i], value)));
            }
            // bucket_cap < BUCKET_CAPACITY라 넘친 한 칸(cap + 1번째)도 페이지에 들어간다.
            b.keys[n] = key;
            b.values[n] = value;
            b.count += 1;
        }
        // 넘치면 나눈다. 키가 한쪽으로 몰리면 그쪽이 또 넘치므로 반복한다(1단계와 같은 loop).
        loop {
            let id = self.bucket_of(key)?;
            let count = {
                let page = self.pool.fetch(id)?;
                bytemuck::cast_ref::<_, BucketPage>(&*page).count as usize
            };
            if count <= self.bucket_cap {
                return Ok(None);
            }
            self.split(id, key)?;
        }
    }

    /// 버킷 `id`(`key`가 든 버킷)를 지역 깊이 d → d+1로 나눈다. 새로 보는 비트 = 해시의 d번째 비트.
    fn split(&mut self, id: PageId, key: u64) -> Result<()> {
        let d = {
            let page = self.pool.fetch(id)?;
            bytemuck::cast_ref::<_, BucketPage>(&*page).local_depth as u32
        };
        if d == self.global_depth {
            self.double()?;
        }
        let bit = 1u64 << d;
        let new_id = {
            let mut old_page = self.pool.fetch_mut(id)?;
            let (new_id, mut new_page) = self.pool.new_page()?; // 핀 2개: 옛 버킷 + 새 버킷
            let old: &mut BucketPage = bytemuck::cast_mut(&mut *old_page);
            let new = BucketPage::init(&mut new_page, (d + 1) as u64);
            old.local_depth = (d + 1) as u64;
            // 새 비트가 0이면 제자리(앞으로 당겨 채움), 1이면 새 버킷 끝에 붙인다.
            let n = old.count as usize;
            let mut stay = 0;
            for r in 0..n {
                let (k, v) = (old.keys[r], old.values[r]);
                if mix(k) & bit == 0 {
                    old.keys[stay] = k;
                    old.values[stay] = v;
                    stay += 1;
                } else {
                    let m = new.count as usize;
                    new.keys[m] = k;
                    new.values[m] = v;
                    new.count += 1;
                }
            }
            old.count = stay as u64;
            new_id
        };
        // 이 버킷을 가리키던 칸 = 하위 d비트가 접미사인 칸, 2^d 간격. 그중 새 비트가 1인 칸
        // (접미사 | bit 부터 2^(d+1) 간격)만 새 버킷으로. 1단계의 디렉터리 전체 훑기 대신
        // 해당 칸만 건너뛰며 고친다 → 건드리는 디렉터리 페이지만 dirty가 된다.
        let suffix = (mix(key) & (bit - 1)) as usize;
        let (len, step) = (1usize << self.global_depth, 1usize << (d + 1));
        let mut s = suffix | bit as usize;
        while s < len {
            let p = s / DIR_SLOTS;
            let mut page = self.pool.fetch_mut(self.dir_pages[p])?;
            let dir: &mut DirectoryPage = bytemuck::cast_mut(&mut *page);
            while s < len && s / DIR_SLOTS == p {
                dir.slots[s % DIR_SLOTS] = new_id;
                s += step;
            }
        }
        Ok(())
    }

    /// 디렉터리를 두 배로: 칸 i와 칸 i + 2^전역은 새 비트만 다르니 같은 버킷을 가리킨다(뒤 절반 = 앞 절반 복사).
    /// 칸이 한 페이지(2048) 안이면 그 페이지 안에서 복사, 넘으면 디렉터리 페이지를 한 장씩 새 페이지로 복사한다.
    fn double(&mut self) -> Result<()> {
        let len = 1usize << self.global_depth;
        if 2 * len > MAX_DIR_PAGES * DIR_SLOTS {
            return Err(PoolError::Io(std::io::Error::other(format!(
                "hash directory full at global depth {}",
                self.global_depth
            ))));
        }
        if len < DIR_SLOTS {
            let mut page = self.pool.fetch_mut(self.dir_pages[0])?;
            let dir: &mut DirectoryPage = bytemuck::cast_mut(&mut *page);
            dir.slots.copy_within(0..len, len);
        } else {
            // len은 2048의 배수 → 페이지 단위로 통째 복사. 쓰기 = 새 디렉터리 페이지 수(카운터에 잡힌다, Q3).
            for p in 0..self.dir_pages.len() {
                let src = self.pool.fetch(self.dir_pages[p])?;
                let (new_id, mut dst) = self.pool.new_page()?; // 핀 2개
                *dst = *src;
                self.dir_pages.push(new_id);
            }
        }
        self.global_depth += 1;
        self.write_meta()
    }

    /// 메모리 사본(전역 깊이·디렉터리 페이지 목록)을 메타 페이지에 반영한다.
    fn write_meta(&self) -> Result<()> {
        let mut page = self.pool.fetch_mut(META)?;
        let m: &mut HashMetaPage = bytemuck::cast_mut(&mut *page);
        m.global_depth = self.global_depth as u64;
        m.dir_count = self.dir_pages.len() as u64;
        m.dir_pages[..self.dir_pages.len()].copy_from_slice(&self.dir_pages);
        Ok(())
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

    /// I3 구조 검사(테스트용 — 공개 `check()`는 diff 11): 디렉터리를 페이지에서 읽어
    /// 버킷마다 가리키는 칸 수 = 2^(전역−지역), 그 칸들의 하위 d비트 = 버킷 키들의 `mix` 접미사.
    /// 반환: 모든 (키, 값).
    fn check_i3(t: &PagedHash<MemStorage>) -> Vec<(u64, u64)> {
        use std::collections::HashMap;
        let g = t.global_depth;
        assert_eq!(
            t.dir_pages.len(),
            (1usize << g).div_ceil(DIR_SLOTS),
            "디렉터리 페이지 수"
        );
        {
            let page = t.pool().fetch(META).unwrap();
            let m: &HashMetaPage = bytemuck::cast_ref(&*page);
            assert_eq!(m.global_depth, g as u64, "메타 global_depth");
            assert_eq!(m.dir_pages(), &t.dir_pages[..], "메타 dir_pages");
        }
        let mut slots_of: HashMap<PageId, Vec<usize>> = HashMap::new();
        for s in 0..1usize << g {
            let page = t.pool().fetch(t.dir_pages[s / DIR_SLOTS]).unwrap();
            let dir: &DirectoryPage = bytemuck::cast_ref(&*page);
            slots_of
                .entry(dir.slots[s % DIR_SLOTS])
                .or_default()
                .push(s);
        }
        let mut all = Vec::new();
        for (&b, slots) in &slots_of {
            let page = t.pool().fetch(b).unwrap();
            let bucket: &BucketPage = bytemuck::cast_ref(&*page);
            let d = bucket.local_depth as u32;
            assert!(d <= g, "버킷 {b} 지역 깊이 {d} > 전역 {g}");
            assert_eq!(slots.len(), 1 << (g - d), "버킷 {b}을 가리키는 칸 수");
            let mask = (1usize << d) - 1;
            let suffix = slots[0] & mask;
            assert!(
                slots.iter().all(|&s| s & mask == suffix),
                "버킷 {b} 칸 접미사 불일치"
            );
            assert!(bucket.keys().len() <= t.bucket_cap, "버킷 {b} 과적");
            for (&k, &v) in bucket.keys().iter().zip(bucket.values()) {
                assert_eq!(
                    mix(k) as usize & mask,
                    suffix,
                    "키 {k}가 버킷 {b}의 접미사와 다르다"
                );
                all.push((k, v));
            }
        }
        assert_eq!(t.pool().pinned_frames(), 0, "핀 누수");
        all
    }

    fn assert_matches(t: &PagedHash<MemStorage>, model: &std::collections::BTreeMap<u64, u64>) {
        let mut all = check_i3(t);
        all.sort_unstable();
        assert_eq!(all, model.iter().map(|(&k, &v)| (k, v)).collect::<Vec<_>>());
        for (&k, &v) in model {
            assert_eq!(t.get(k).unwrap(), Some(v), "get({k})");
        }
    }

    #[test]
    fn insert_is_upsert() {
        let mut t = hash(2, 2);
        assert_eq!(t.insert(7, 70).unwrap(), None);
        assert_eq!(t.insert(7, 71).unwrap(), Some(70));
        assert_eq!(t.get(7).unwrap(), Some(71));
        assert_eq!(t.pool().pinned_frames(), 0);
    }

    #[test]
    fn first_overflow_doubles_directory_inside_page_and_splits() {
        let mut t = hash(2, 1);
        t.insert(1, 10).unwrap();
        assert_eq!(t.global_depth(), 0);
        t.insert(2, 20).unwrap(); // 버킷 용량 1 초과 → 두 배 + 분할(키가 갈릴 때까지 반복)
        assert!(t.global_depth() >= 1);
        assert_eq!(t.dir_pages.len(), 1, "칸 2048개 이하는 디렉터리 페이지 1장");
        let model = [(1, 10), (2, 20)].into_iter().collect();
        assert_matches(&t, &model);
    }

    /// 버킷 용량 2·프레임 2(최소)로 수천 개 → 전역 깊이가 11을 넘어 디렉터리가 여러 페이지로 자란다.
    #[test]
    fn many_keys_with_two_frames_grow_directory_past_one_page() {
        let mut t = hash(2, 2);
        let mut model = std::collections::BTreeMap::new();
        let mut seed = 1u64;
        for i in 0..6000u64 {
            seed = mix(seed.wrapping_add(i));
            let k = seed % 20_000;
            assert_eq!(t.insert(k, i).unwrap(), model.insert(k, i), "insert({k})");
            assert_eq!(t.pool().pinned_frames(), 0);
        }
        assert!(t.global_depth() > 11, "전역 깊이 {}", t.global_depth());
        assert!(t.dir_pages.len() >= 2);
        assert_matches(&t, &model);
        assert_eq!(t.get(20_001).unwrap(), None);
    }

    /// 매 삽입마다 I3 — 작은 수에서 두 배·분할·반복 분할의 모든 중간 상태를 본다.
    #[test]
    fn every_intermediate_state_satisfies_i3() {
        for cap in 1..=3 {
            let mut t = hash(2, cap);
            let mut model = std::collections::BTreeMap::new();
            for k in 0..120u64 {
                t.insert(k * 7, k).unwrap();
                model.insert(k * 7, k);
                assert_matches(&t, &model);
            }
        }
    }

    /// 칸 2048 → 4096 두 배는 디렉터리 페이지를 통째로 새 페이지에 복사한다 → flush 때 쓰기로 잡힌다.
    #[test]
    fn doubling_across_a_page_writes_the_new_directory_page() {
        let mut t = hash(4, 2);
        let mut k = 0u64;
        while t.global_depth() < 11 {
            t.insert(k, k).unwrap();
            k += 1;
        }
        t.pool().flush().unwrap();
        let before = t.dir_pages.clone();
        while t.global_depth() < 12 {
            t.insert(k, k).unwrap();
            k += 1;
        }
        assert_eq!(t.dir_pages.len(), 2);
        assert_eq!(t.dir_pages[0], before[0]);
        let new_dir = t.dir_pages[1];
        assert!(
            t.pool().is_dirty(new_dir) || !t.pool().is_resident(new_dir),
            "새 디렉터리 페이지는 써야 한다"
        );
        let (_, w0) = t.pool().io_counts();
        t.pool().flush().unwrap();
        let (_, w1) = t.pool().io_counts();
        assert!(w1 > w0);
        let model: std::collections::BTreeMap<u64, u64> = (0..k).map(|k| (k, k)).collect();
        assert_matches(&t, &model);
    }
}

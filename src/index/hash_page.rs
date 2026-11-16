//! 페이지 Extendible Hash의 디스크 레이아웃 — 메타 → 디렉터리 → 버킷 3단(S3), 각 16KB `#[repr(C)]` Pod 구조체.
//!
//! 칸 번호 `slot`(= `mix(key)`의 하위 `global_depth` 비트)은 디렉터리 페이지 `slot / DIR_SLOTS`의
//! `slot % DIR_SLOTS`번째 칸에 있다. 메타가 디렉터리 페이지 번호를 차례로 들고, 디렉터리 칸은 버킷 페이지 번호를 든다.

use bytemuck::{Pod, Zeroable};

use crate::storage::{PAGE_SIZE, Page, PageId};

/// 파일 앞머리 표식 — B+tree 파일(`IDXBTREE`)을 Hash로 여는 실수를 막는다.
pub const HASH_MAGIC: u64 = u64::from_le_bytes(*b"IDXHASH_");

/// 디렉터리 페이지 한 장의 칸 수 — 헤더 없이 페이지 전체(2048칸, S3).
pub const DIR_SLOTS: usize = PAGE_SIZE / 8;
/// 메타가 들 수 있는 디렉터리 페이지 수: 2048 − 헤더 4.
pub const MAX_DIR_PAGES: usize = PAGE_SIZE / 8 - 4;
/// 버킷 한 장에 들어가는 키 수: (2048 − 헤더 2) / 2 (키·값 한 쌍씩).
pub const BUCKET_CAPACITY: usize = (PAGE_SIZE / 8 - 2) / 2;

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
pub struct HashMetaPage {
    pub magic: u64,
    /// 디렉터리 칸 번호에 쓰는 해시 하위 비트 수. 칸 수 = 2^global_depth.
    pub global_depth: u64,
    /// 버킷 하나의 최대 키 수(1단계 `bucket_cap`, 용량 미만).
    pub bucket_cap: u64,
    /// 쓰이는 디렉터리 페이지 수 = ceil(2^global_depth / DIR_SLOTS).
    pub dir_count: u64,
    pub dir_pages: [PageId; MAX_DIR_PAGES],
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
pub struct DirectoryPage {
    /// 칸마다 버킷 페이지 번호. 여러 칸이 한 버킷을 같이 가리킬 수 있다.
    pub slots: [PageId; DIR_SLOTS],
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
pub struct BucketPage {
    /// 이 버킷이 구분에 쓰는 하위 비트 수(≤ 전역 깊이, I3).
    pub local_depth: u64,
    pub count: u64,
    /// `keys[i]`의 값 = `values[i]`. 순서 없음.
    pub keys: [u64; BUCKET_CAPACITY],
    pub values: [u64; BUCKET_CAPACITY],
}

// 패딩이 있으면 derive(Pod)가, 크기가 어긋나면 이 단언이 컴파일 오류를 낸다.
const _: () = assert!(size_of::<HashMetaPage>() == PAGE_SIZE);
const _: () = assert!(size_of::<DirectoryPage>() == PAGE_SIZE);
const _: () = assert!(size_of::<BucketPage>() == PAGE_SIZE);
// 디렉터리 최대 칸 수 2044 × 2048 ≥ 2^21 → 전역 깊이 21까지 담는다(diff 10의 두 배 상한).
const _: () = assert!(MAX_DIR_PAGES * DIR_SLOTS >= 1 << 21);

impl HashMetaPage {
    pub fn init(page: &mut Page, bucket_cap: u64, first_dir: PageId) -> &mut HashMetaPage {
        let m: &mut HashMetaPage = bytemuck::cast_mut(page);
        *m = HashMetaPage::zeroed();
        m.magic = HASH_MAGIC;
        m.bucket_cap = bucket_cap;
        m.dir_count = 1;
        m.dir_pages[0] = first_dir;
        m
    }

    /// 쓰이는 디렉터리 페이지 번호만. `dir_count`가 용량을 넘는 손상 페이지면 슬라이스 범위 panic.
    pub fn dir_pages(&self) -> &[PageId] {
        &self.dir_pages[..self.dir_count as usize]
    }
}

impl BucketPage {
    pub fn init(page: &mut Page, local_depth: u64) -> &mut BucketPage {
        let b: &mut BucketPage = bytemuck::cast_mut(page);
        *b = BucketPage::zeroed();
        b.local_depth = local_depth;
        b
    }

    pub fn keys(&self) -> &[u64] {
        &self.keys[..self.count as usize]
    }

    pub fn values(&self) -> &[u64] {
        &self.values[..self.count as usize]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn capacities_fill_the_page() {
        assert_eq!(DIR_SLOTS, 2048);
        assert_eq!(MAX_DIR_PAGES, 2044);
        assert_eq!(BUCKET_CAPACITY, 1023);
        assert_eq!(2 + 2 * BUCKET_CAPACITY, PAGE_SIZE / 8);
    }

    #[test]
    fn hash_magic_differs_from_btree_magic() {
        assert_ne!(HASH_MAGIC, super::super::btree_page::BTREE_MAGIC);
    }
}

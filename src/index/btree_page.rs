//! 페이지 B+tree의 디스크 레이아웃 — 메타·리프·내부 페이지를 16KB(`u64` 2048칸)에 맞춘 `#[repr(C)]` Pod 구조체.
//!
//! 페이지 0은 항상 메타 페이지라 노드가 될 수 없다 → 페이지 번호 0을 "링크 없음"(`NO_PAGE`)으로 쓴다.
//! 노드 페이지 첫 칸 `kind`로 리프/내부를 구분한다(자식을 fetch했을 때 무엇인지 알아야 하므로).

use bytemuck::{Pod, Zeroable};

use crate::storage::{PAGE_SIZE, Page, PageId};

/// 메타 페이지 번호이자 "링크 없음" 표시. 노드는 1번 페이지부터.
pub const NO_PAGE: PageId = 0;

/// 리프 한 장에 들어가는 키 수: (2048 − 헤더 4) / 2 (키·값 한 쌍씩).
pub const LEAF_CAPACITY: usize = (PAGE_SIZE / 8 - 4) / 2;
/// 내부 노드 한 장에 들어가는 키 수: 헤더 2 + 키 m + 자식 m+1 ≤ 2048 → m = 1022 (1칸 남음).
pub const INTERNAL_CAPACITY: usize = (PAGE_SIZE / 8 - 2 - 1) / 2;

pub const KIND_LEAF: u64 = 1;
pub const KIND_INTERNAL: u64 = 2;

/// 파일 앞머리 표식 — 다른 파일(또는 Hash 인덱스 파일)을 B+tree로 여는 실수를 막는다.
pub const BTREE_MAGIC: u64 = u64::from_le_bytes(*b"IDXBTREE");

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
pub struct MetaPage {
    pub magic: u64,
    pub root: PageId,
    /// 루트에서 리프까지 내려가는 단계 수. 리프 하나뿐이면 0.
    pub height: u64,
    /// 노드 하나의 최대 키 수(1단계 `max_keys`, S2 — 용량 이하).
    pub max_keys: u64,
    _reserved: [u64; PAGE_SIZE / 8 - 4],
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
pub struct LeafPage {
    pub kind: u64,
    pub count: u64,
    pub prev: PageId,
    pub next: PageId,
    pub keys: [u64; LEAF_CAPACITY],
    pub values: [u64; LEAF_CAPACITY],
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
pub struct InternalPage {
    pub kind: u64,
    /// 키 수. 자식 수 = `count + 1`.
    pub count: u64,
    pub keys: [u64; INTERNAL_CAPACITY],
    pub children: [PageId; INTERNAL_CAPACITY + 1],
    _reserved: u64,
}

// 패딩이 있으면 derive(Pod)가, 크기가 어긋나면 이 단언이 컴파일 오류를 낸다.
const _: () = assert!(size_of::<MetaPage>() == PAGE_SIZE);
const _: () = assert!(size_of::<LeafPage>() == PAGE_SIZE);
const _: () = assert!(size_of::<InternalPage>() == PAGE_SIZE);

impl MetaPage {
    pub fn init(page: &mut Page, root: PageId, max_keys: u64) -> &mut MetaPage {
        let m: &mut MetaPage = bytemuck::cast_mut(page);
        *m = MetaPage::zeroed();
        m.magic = BTREE_MAGIC;
        m.root = root;
        m.max_keys = max_keys;
        m
    }
}

impl LeafPage {
    /// 빈 리프로 초기화(링크 없음).
    pub fn init(page: &mut Page) -> &mut LeafPage {
        let l: &mut LeafPage = bytemuck::cast_mut(page);
        *l = LeafPage::zeroed();
        l.kind = KIND_LEAF;
        l
    }

    /// 쓰인 부분만. `count`가 용량을 넘는 손상 페이지면 슬라이스 범위 panic으로 드러난다.
    pub fn keys(&self) -> &[u64] {
        &self.keys[..self.count as usize]
    }

    pub fn values(&self) -> &[u64] {
        &self.values[..self.count as usize]
    }

    /// `i` 자리에 끼우고 뒤를 한 칸씩 민다. 호출자가 `count < LEAF_CAPACITY`를 보장한다.
    pub fn insert_at(&mut self, i: usize, key: u64, value: u64) {
        let n = self.count as usize;
        self.keys.copy_within(i..n, i + 1);
        self.values.copy_within(i..n, i + 1);
        self.keys[i] = key;
        self.values[i] = value;
        self.count += 1;
    }
}

impl InternalPage {
    pub fn init(page: &mut Page) -> &mut InternalPage {
        let n: &mut InternalPage = bytemuck::cast_mut(page);
        *n = InternalPage::zeroed();
        n.kind = KIND_INTERNAL;
        n
    }

    pub fn keys(&self) -> &[u64] {
        &self.keys[..self.count as usize]
    }

    pub fn children(&self) -> &[PageId] {
        &self.children[..self.count as usize + 1]
    }

    /// 구분자를 키 `i` 자리에, 그 오른쪽 자식을 자식 `i + 1` 자리에 끼운다.
    pub fn insert_at(&mut self, i: usize, sep: u64, right: PageId) {
        let n = self.count as usize;
        self.keys.copy_within(i..n, i + 1);
        self.children.copy_within(i + 1..n + 1, i + 2);
        self.keys[i] = sep;
        self.children[i + 1] = right;
        self.count += 1;
    }
}

/// 노드 페이지의 종류. 메타·빈 페이지·손상 페이지면 `None`.
pub fn kind_of(page: &Page) -> Option<u64> {
    let kind: &u64 = bytemuck::from_bytes(&page.0[..8]);
    matches!(*kind, KIND_LEAF | KIND_INTERNAL).then_some(*kind)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::Storage;
    use crate::storage::file::FileStorage;

    #[test]
    fn capacities_fill_the_page() {
        assert_eq!(LEAF_CAPACITY, 1022);
        assert_eq!(INTERNAL_CAPACITY, 1022);
        // 리프: 4 + 2×1022 = 2048칸, 내부: 2 + 1022 + 1023 + 1(여분) = 2048칸.
        assert_eq!(4 + 2 * LEAF_CAPACITY, PAGE_SIZE / 8);
        assert_eq!(
            2 + INTERNAL_CAPACITY + (INTERNAL_CAPACITY + 1) + 1,
            PAGE_SIZE / 8
        );
    }

    #[test]
    fn kind_tells_leaf_from_internal_and_rejects_others() {
        let mut p = Page::zeroed();
        assert_eq!(kind_of(&p), None, "0으로 찬 페이지는 노드가 아니다");
        LeafPage::init(&mut p);
        assert_eq!(kind_of(&p), Some(KIND_LEAF));
        InternalPage::init(&mut p);
        assert_eq!(kind_of(&p), Some(KIND_INTERNAL));
        MetaPage::init(&mut p, 1, 4);
        assert_eq!(kind_of(&p), None, "메타 페이지는 노드가 아니다");
    }

    #[test]
    fn pages_round_trip_through_a_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("t.db");
        {
            let mut s = FileStorage::open(&path).unwrap();
            let mut p = Page::zeroed();
            MetaPage::init(&mut p, 2, 4).height = 1;
            s.write_page(0, &p).unwrap();

            let mut p = Page::zeroed();
            let l = LeafPage::init(&mut p);
            l.count = LEAF_CAPACITY as u64; // 꽉 채워 마지막 칸까지 왕복 확인
            for i in 0..LEAF_CAPACITY {
                l.keys[i] = i as u64 * 10;
                l.values[i] = i as u64 * 10 + 1;
            }
            l.next = 3;
            s.write_page(1, &p).unwrap();

            let mut p = Page::zeroed();
            let n = InternalPage::init(&mut p);
            n.count = 1;
            n.keys[0] = 50;
            n.children[0] = 1;
            n.children[1] = 3;
            s.write_page(2, &p).unwrap();
        } // 닫힘

        let mut s = FileStorage::open(&path).unwrap();
        let mut p = Page::zeroed();
        s.read_page(0, &mut p).unwrap();
        let m: &MetaPage = bytemuck::cast_ref(&p);
        assert_eq!(
            (m.magic, m.root, m.height, m.max_keys),
            (BTREE_MAGIC, 2, 1, 4)
        );

        s.read_page(1, &mut p).unwrap();
        let l: &LeafPage = bytemuck::cast_ref(&p);
        assert_eq!(
            (l.kind, l.keys().len(), l.prev, l.next),
            (KIND_LEAF, LEAF_CAPACITY, NO_PAGE, 3)
        );
        assert_eq!(l.keys()[LEAF_CAPACITY - 1], (LEAF_CAPACITY as u64 - 1) * 10);
        assert_eq!(
            l.values()[LEAF_CAPACITY - 1],
            (LEAF_CAPACITY as u64 - 1) * 10 + 1
        );

        s.read_page(2, &mut p).unwrap();
        let n: &InternalPage = bytemuck::cast_ref(&p);
        assert_eq!(
            (n.kind, n.keys(), n.children()),
            (KIND_INTERNAL, &[50][..], &[1, 3][..])
        );
    }
}

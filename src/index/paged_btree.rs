//! 페이지 B+tree — 1단계 `btree.rs`의 알고리즘을 버퍼풀 위 16KB 페이지로 옮긴 것.
//!
//! 핀 규칙(S5·P6): 내려갈 때 조상 페이지를 쥐고 있지 않는다. 자식에서 분할이 올라오면 부모를 **다시 fetch**한다.
//! 한 연산이 동시에 쥐는 핀은 최대 2개(분할 중인 노드 + 새 형제) — 트리 높이와 무관.

use crate::storage::pool::{BufferPool, PoolError, Result};
use crate::storage::{PageId, Storage};

use super::btree_page::{
    INTERNAL_CAPACITY, InternalPage, KIND_INTERNAL, KIND_LEAF, LEAF_CAPACITY, LeafPage, MetaPage,
    NO_PAGE, kind_of,
};

/// 노드가 갈라졌을 때 부모에 올릴 것: (구분자, 새 오른쪽 노드).
type Split = (u64, PageId);

const META: PageId = 0;

pub struct PagedBTree<S: Storage> {
    pool: BufferPool<S>,
    root: PageId,
    height: u64,
    max_keys: usize,
}

impl<S: Storage> PagedBTree<S> {
    /// 빈 저장소 위에 새 트리를 만든다: 페이지 0 = 메타, 페이지 1 = 빈 루트 리프.
    ///
    /// 삽입은 "넣고 넘치면 나눈다"라 노드가 잠깐 `max_keys + 1`개를 담는다 → 그 한 칸까지 페이지에 들어가야 한다.
    pub fn create(pool: BufferPool<S>, max_keys: usize) -> Result<Self> {
        assert!(max_keys >= 2, "max_keys는 2 이상이어야 한다");
        assert!(
            max_keys < LEAF_CAPACITY.min(INTERNAL_CAPACITY),
            "max_keys + 1이 페이지 용량({LEAF_CAPACITY})을 넘는다"
        );
        let (meta_id, mut meta) = pool.new_page()?;
        assert_eq!(
            meta_id, META,
            "create는 빈 저장소에서만 — 첫 페이지가 메타여야 한다"
        );
        let (root, mut leaf) = pool.new_page()?;
        LeafPage::init(&mut leaf);
        MetaPage::init(&mut meta, root, max_keys as u64);
        drop((meta, leaf));
        Ok(Self {
            pool,
            root,
            height: 0,
            max_keys,
        })
    }

    pub fn get(&self, key: u64) -> Result<Option<u64>> {
        let mut id = self.root;
        loop {
            let page = self.pool.fetch(id)?; // 한 번에 한 장만 쥔다
            match kind_of(&page) {
                Some(KIND_INTERNAL) => {
                    let node: &InternalPage = bytemuck::cast_ref(&*page);
                    // key 이하인 구분자 개수 = 내려갈 자식 칸 (key == 구분자면 오른쪽으로)
                    id = node.children()[node.keys().partition_point(|&sep| sep <= key)];
                }
                Some(KIND_LEAF) => {
                    let leaf: &LeafPage = bytemuck::cast_ref(&*page);
                    return Ok(leaf
                        .keys()
                        .binary_search(&key)
                        .ok()
                        .map(|i| leaf.values()[i]));
                }
                _ => return Err(corrupt(id)),
            }
        }
    }

    /// 같은 키가 있으면 값을 바꾸고 이전 값을 돌려준다(upsert).
    pub fn insert(&mut self, key: u64, value: u64) -> Result<Option<u64>> {
        let (old, split) = self.insert_at(self.root, key, value)?;
        if let Some((sep, right)) = split {
            // 루트가 갈라졌다 → 새 루트를 맨 위에 올린다. 높이는 여기서만 1 늘어난다.
            let (new_root, mut page) = self.pool.new_page()?;
            let node = InternalPage::init(&mut page);
            node.count = 1;
            node.keys[0] = sep;
            node.children[0] = self.root;
            node.children[1] = right;
            drop(page);
            self.root = new_root;
            self.height += 1;
            let mut meta = self.pool.fetch_mut(META)?;
            let m: &mut MetaPage = bytemuck::cast_mut(&mut *meta);
            m.root = new_root;
            m.height = self.height;
        }
        Ok(old)
    }

    pub fn root(&self) -> PageId {
        self.root
    }

    pub fn height(&self) -> u64 {
        self.height
    }

    pub fn pool(&self) -> &BufferPool<S> {
        &self.pool
    }

    /// `id` 아래에 넣는다. 반환 = (이전 값, `id`가 갈라졌으면 그 `Split`).
    fn insert_at(
        &mut self,
        id: PageId,
        key: u64,
        value: u64,
    ) -> Result<(Option<u64>, Option<Split>)> {
        let kind = kind_of(&*self.pool.fetch(id)?); // 종류만 보고 바로 놓는다
        match kind {
            Some(KIND_LEAF) => self.insert_into_leaf(id, key, value),
            Some(KIND_INTERNAL) => {
                let (i, child) = {
                    let page = self.pool.fetch(id)?;
                    let node: &InternalPage = bytemuck::cast_ref(&*page);
                    let i = node.keys().partition_point(|&sep| sep <= key);
                    (i, node.children()[i])
                }; // 부모 핀을 여기서 놓고 내려간다(S5)
                let (old, split) = self.insert_at(child, key, value)?;
                let Some((sep, right)) = split else {
                    return Ok((old, None));
                };
                // 갈라진 자식 바로 오른쪽 칸에 구분자와 새 자식을 끼운다 — 부모를 다시 fetch.
                let mut page = self.pool.fetch_mut(id)?;
                let node: &mut InternalPage = bytemuck::cast_mut(&mut *page);
                node.insert_at(i, sep, right);
                if node.count as usize > self.max_keys {
                    unimplemented!("내부 노드 분할(push-up)은 diff 07");
                }
                Ok((old, None))
            }
            _ => Err(corrupt(id)),
        }
    }

    fn insert_into_leaf(
        &mut self,
        id: PageId,
        key: u64,
        value: u64,
    ) -> Result<(Option<u64>, Option<Split>)> {
        let mut page = self.pool.fetch_mut(id)?;
        let leaf: &mut LeafPage = bytemuck::cast_mut(&mut *page);
        match leaf.keys().binary_search(&key) {
            Ok(i) => return Ok((Some(std::mem::replace(&mut leaf.values[i], value)), None)),
            Err(i) => leaf.insert_at(i, key, value),
        }
        if leaf.count as usize <= self.max_keys {
            return Ok((None, None));
        }
        drop(page);
        Ok((None, Some(self.split_leaf(id)?)))
    }

    /// 넘친 리프 A를 반으로 나눠 오른쪽 절반을 새 리프 A'로 옮긴다.
    /// 구분자 = A'의 첫 키를 복사(copy-up — 리프에도 남는다).
    /// 연결: A ⇄ C  →  A ⇄ A' ⇄ C (C의 prev까지 고친다).
    fn split_leaf(&mut self, id: PageId) -> Result<Split> {
        let (new_id, sep, old_next) = {
            let mut left_page = self.pool.fetch_mut(id)?;
            let (new_id, mut right_page) = self.pool.new_page()?; // 핀 2개: A + A'
            let left: &mut LeafPage = bytemuck::cast_mut(&mut *left_page);
            let right = LeafPage::init(&mut right_page);
            let n = left.count as usize;
            let mid = n / 2;
            right.keys[..n - mid].copy_from_slice(&left.keys[mid..n]);
            right.values[..n - mid].copy_from_slice(&left.values[mid..n]);
            right.count = (n - mid) as u64;
            left.count = mid as u64;
            let old_next = left.next;
            right.prev = id;
            right.next = old_next;
            left.next = new_id; // A.next = A'
            (new_id, right.keys[0], old_next)
        }; // A·A' 핀을 놓은 뒤 C를 잡는다 — 동시 핀 2개 유지
        if old_next != NO_PAGE {
            let mut c = self.pool.fetch_mut(old_next)?;
            let c: &mut LeafPage = bytemuck::cast_mut(&mut *c);
            c.prev = new_id; // C.prev = A'
        }
        Ok((sep, new_id))
    }
}

fn corrupt(id: PageId) -> PoolError {
    PoolError::Io(std::io::Error::new(
        std::io::ErrorKind::InvalidData,
        format!("page {id} is not a B+tree node"),
    ))
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use super::*;
    use crate::storage::MemStorage;
    use crate::storage::pager::Pager;

    fn tree(frames: usize, max_keys: usize) -> PagedBTree<MemStorage> {
        let pool = BufferPool::new(Pager::new(MemStorage::new()).unwrap(), frames);
        PagedBTree::create(pool, max_keys).unwrap()
    }

    /// 맨 왼쪽 리프부터 next를 따라 (키 목록, 각 리프의 (prev, 자기, next)).
    fn walk_leaves(t: &PagedBTree<MemStorage>) -> (Vec<u64>, Vec<(PageId, PageId, PageId)>) {
        let mut id = t.root();
        loop {
            let page = t.pool().fetch(id).unwrap();
            if kind_of(&page) == Some(KIND_LEAF) {
                break;
            }
            let node: &InternalPage = bytemuck::cast_ref(&*page);
            id = node.children()[0];
        }
        let (mut keys, mut links) = (Vec::new(), Vec::new());
        while id != NO_PAGE {
            let page = t.pool().fetch(id).unwrap();
            let leaf: &LeafPage = bytemuck::cast_ref(&*page);
            keys.extend_from_slice(leaf.keys());
            links.push((leaf.prev, id, leaf.next));
            id = leaf.next;
        }
        (keys, links)
    }

    #[test]
    fn empty_tree_finds_nothing() {
        let t = tree(2, 4);
        assert_eq!(t.get(7).unwrap(), None);
        assert_eq!((t.root(), t.height()), (1, 0));
    }

    #[test]
    fn upsert_inside_one_leaf() {
        let mut t = tree(2, 4);
        assert_eq!(t.insert(5, 50).unwrap(), None);
        assert_eq!(t.insert(5, 55).unwrap(), Some(50));
        assert_eq!(t.get(5).unwrap(), Some(55));
    }

    #[test]
    fn leaf_split_copies_separator_up_and_links_siblings() {
        let mut t = tree(2, 4); // 프레임 2개 — 동시 핀 2개 상한(P6) 그대로
        for k in [10, 20, 30, 40, 50] {
            t.insert(k, k * 10).unwrap();
        }
        // 5개 → 넘침 → [10,20] | [30,40,50], 구분자 30이 새 루트로 복사
        assert_eq!(t.height(), 1);
        {
            let page = t.pool().fetch(t.root()).unwrap();
            let root: &InternalPage = bytemuck::cast_ref(&*page);
            assert_eq!(root.keys(), &[30]);
        }
        let (keys, links) = walk_leaves(&t);
        assert_eq!(keys, vec![10, 20, 30, 40, 50]);
        let [(p0, a, n0), (p1, b, n1)] = links[..] else {
            panic!("리프 2개")
        };
        assert_eq!((p0, n0, p1, n1), (NO_PAGE, b, a, NO_PAGE));
        assert_eq!(
            t.get(30).unwrap(),
            Some(300),
            "구분자와 같은 키는 오른쪽에서 찾는다"
        );
    }

    #[test]
    fn middle_split_fixes_right_neighbours_prev() {
        let mut t = tree(2, 4);
        for k in [10, 20, 30, 40, 50] {
            t.insert(k, k).unwrap(); // [10,20] | [30,40,50]
        }
        for k in [11, 12, 13] {
            t.insert(k, k).unwrap(); // 왼쪽 리프가 다시 넘쳐 가운데에 새 리프
        }
        let (keys, links) = walk_leaves(&t);
        assert_eq!(keys, vec![10, 11, 12, 13, 20, 30, 40, 50]);
        assert_eq!(links.len(), 3);
        for w in links.windows(2) {
            assert_eq!(w[0].2, w[1].1, "a.next = b");
            assert_eq!(w[1].0, w[0].1, "b.prev = a");
        }
    }

    #[test]
    fn matches_btreemap_until_root_is_full() {
        // max_keys 4: 루트 내부 노드가 키 4개(리프 5장)를 넘기 전까지 — 내부 분할은 diff 07.
        let mut t = tree(2, 4);
        let mut model = BTreeMap::new();
        for k in [7u64, 3, 9, 1, 5, 8, 2, 6, 4, 0, 7, 3] {
            assert_eq!(t.insert(k, k + 100).unwrap(), model.insert(k, k + 100));
        }
        for k in 0..12 {
            assert_eq!(t.get(k).unwrap(), model.get(&k).copied(), "key {k}");
        }
        assert_eq!(walk_leaves(&t).0, model.keys().copied().collect::<Vec<_>>());
        assert_eq!(t.pool().pinned_frames(), 0, "핀 누수 없음");
    }

    #[test]
    #[should_panic(expected = "페이지 용량")]
    fn max_keys_must_leave_one_slot_for_overflow() {
        tree(2, LEAF_CAPACITY);
    }
}

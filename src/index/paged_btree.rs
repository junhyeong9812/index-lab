//! 페이지 B+tree — 1단계 `btree.rs`의 알고리즘을 버퍼풀 위 16KB 페이지로 옮긴 것.
//!
//! 핀 규칙(S5·P6): 내려갈 때 조상 페이지를 쥐고 있지 않는다. 자식에서 분할이 올라오면 부모를 **다시 fetch**한다.
//! 한 연산이 동시에 쥐는 핀은 최대 2개(분할 중인 노드 + 새 형제) — 트리 높이와 무관.

use crate::storage::pool::{BufferPool, PoolError, Result};
use crate::storage::{PageId, Storage};

use super::btree_page::{
    BTREE_MAGIC, INTERNAL_CAPACITY, InternalPage, KIND_INTERNAL, KIND_LEAF, LEAF_CAPACITY,
    LeafPage, MetaPage, NO_PAGE, kind_of,
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
                if node.count as usize <= self.max_keys {
                    return Ok((old, None));
                }
                drop(page); // 부모 핀을 놓고 분할 — split_internal이 다시 잡는다(핀 ≤ 2)
                Ok((old, Some(self.split_internal(id)?)))
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

    /// 넘친 내부 노드를 가운데 키로 나눈다. 가운데 키는 부모로 **이동**한다(push-up — 아래에 남지 않는다).
    /// keys [k0 .. k(mid-1)] · k(mid) · [k(mid+1) ..] → 왼쪽(id) · 부모로 · 오른쪽(새 노드)
    /// 자식은 mid+1개가 왼쪽에 남고 나머지가 오른쪽으로 간다. 내부 노드엔 형제 링크가 없어 이웃은 안 건드린다.
    fn split_internal(&mut self, id: PageId) -> Result<Split> {
        let mut left_page = self.pool.fetch_mut(id)?;
        let (new_id, mut right_page) = self.pool.new_page()?; // 핀 2개: 왼쪽 + 새 오른쪽
        let left: &mut InternalPage = bytemuck::cast_mut(&mut *left_page);
        let right = InternalPage::init(&mut right_page);
        let n = left.count as usize;
        let mid = n / 2;
        let sep = left.keys[mid];
        right.keys[..n - mid - 1].copy_from_slice(&left.keys[mid + 1..n]);
        right.children[..n - mid].copy_from_slice(&left.children[mid + 1..=n]);
        right.count = (n - mid - 1) as u64;
        left.count = mid as u64; // keys[mid]는 왼쪽에서 빠진다
        Ok((sep, new_id))
    }

    /// `key`가 있어야 할 리프 번호. 한 번에 한 장만 쥔다.
    fn find_leaf(&self, key: u64) -> Result<PageId> {
        let mut id = self.root;
        loop {
            let page = self.pool.fetch(id)?;
            match kind_of(&page) {
                Some(KIND_INTERNAL) => {
                    let node: &InternalPage = bytemuck::cast_ref(&*page);
                    id = node.children()[node.keys().partition_point(|&sep| sep <= key)];
                }
                Some(KIND_LEAF) => return Ok(id),
                _ => return Err(corrupt(id)),
            }
        }
    }

    /// 리프 `id`를 잠깐 쥐고 `f`로 읽는다(핀 1개). 리프가 아니면 손상.
    fn with_leaf<T>(&self, id: PageId, f: impl FnOnce(&LeafPage) -> T) -> Result<T> {
        let page = self.pool.fetch(id)?;
        if kind_of(&page) != Some(KIND_LEAF) {
            return Err(corrupt(id));
        }
        Ok(f(bytemuck::cast_ref(&*page)))
    }

    /// `lo <= key <= hi`를 오름차순으로. lo가 있어야 할 리프에서 시작해 `next`만 따라간다 — 핀은 한 번에 1장.
    pub fn range(&self, lo: u64, hi: u64) -> Result<Vec<(u64, u64)>> {
        let mut out = Vec::new();
        if lo > hi {
            return Ok(out);
        }
        let mut id = self.find_leaf(lo)?;
        while id != NO_PAGE {
            id = self.with_leaf(id, |leaf| {
                let start = leaf.keys().partition_point(|&k| k < lo);
                for (&k, &v) in leaf.keys()[start..].iter().zip(&leaf.values()[start..]) {
                    if k > hi {
                        return NO_PAGE; // 범위 끝 — 더 갈 필요 없음
                    }
                    out.push((k, v));
                }
                leaf.next
            })?;
        }
        Ok(out)
    }

    /// `range`와 같은 범위를 내림차순으로. hi가 있어야 할 리프에서 시작해 `prev`만 따라간다.
    pub fn range_rev(&self, lo: u64, hi: u64) -> Result<Vec<(u64, u64)>> {
        let mut out = Vec::new();
        if lo > hi {
            return Ok(out);
        }
        let mut id = self.find_leaf(hi)?;
        while id != NO_PAGE {
            id = self.with_leaf(id, |leaf| {
                let end = leaf.keys().partition_point(|&k| k <= hi);
                for (&k, &v) in leaf.keys()[..end].iter().zip(&leaf.values()[..end]).rev() {
                    if k < lo {
                        return NO_PAGE;
                    }
                    out.push((k, v));
                }
                leaf.prev
            })?;
        }
        Ok(out)
    }

    /// 이미 만들어진 저장소를 다시 연다 — 루트·높이·`max_keys`는 메타 페이지(0)에서 읽는다.
    /// 메타가 아니거나(`magic`) `max_keys`가 `create` 조건을 벗어나면 `InvalidData`.
    pub fn open(pool: BufferPool<S>) -> Result<Self> {
        let (magic, root, height, max_keys) = {
            let page = pool.fetch(META)?;
            let m: &MetaPage = bytemuck::cast_ref(&*page);
            (m.magic, m.root, m.height, m.max_keys as usize)
        };
        if magic != BTREE_MAGIC {
            return Err(invalid("page 0 is not a B+tree meta page".into()));
        }
        if !(2..LEAF_CAPACITY.min(INTERNAL_CAPACITY)).contains(&max_keys) {
            return Err(invalid(format!("meta max_keys {max_keys} out of range")));
        }
        Ok(Self {
            pool,
            root,
            height,
            max_keys,
        })
    }
}

/// 1단계 `Index` 계약엔 `Result`가 없다 → I/O·손상은 여기서만 panic(명세 Q6).
impl<S: Storage> super::Index for PagedBTree<S> {
    fn insert(&mut self, key: u64, value: u64) -> Option<u64> {
        PagedBTree::insert(self, key, value).expect("paged B+tree insert")
    }

    fn get(&self, key: u64) -> Option<u64> {
        PagedBTree::get(self, key).expect("paged B+tree get")
    }

    fn range(&self, lo: u64, hi: u64) -> Vec<(u64, u64)> {
        PagedBTree::range(self, lo, hi).expect("paged B+tree range")
    }

    fn range_rev(&self, lo: u64, hi: u64) -> Vec<(u64, u64)> {
        PagedBTree::range_rev(self, lo, hi).expect("paged B+tree range_rev")
    }
}

fn invalid(msg: String) -> PoolError {
    PoolError::Io(std::io::Error::new(std::io::ErrorKind::InvalidData, msg))
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

    /// 모든 내부 노드의 키를 모은다(구조 관찰용).
    fn internal_keys(t: &PagedBTree<MemStorage>) -> Vec<u64> {
        let (mut out, mut stack) = (Vec::new(), vec![t.root()]);
        while let Some(id) = stack.pop() {
            let page = t.pool().fetch(id).unwrap();
            if kind_of(&page) == Some(KIND_INTERNAL) {
                let node: &InternalPage = bytemuck::cast_ref(&*page);
                out.extend_from_slice(node.keys());
                stack.extend_from_slice(node.children());
            }
        }
        out
    }

    fn model_range(m: &BTreeMap<u64, u64>, lo: u64, hi: u64) -> Vec<(u64, u64)> {
        if lo > hi {
            return Vec::new();
        }
        m.range(lo..=hi).map(|(&k, &v)| (k, v)).collect()
    }

    #[test]
    fn internal_split_pushes_middle_key_up() {
        let mut t = tree(2, 2);
        for k in 1..=20 {
            t.insert(k, k).unwrap();
        }
        assert!(t.height() >= 2, "내부 노드도 갈라져 루트가 위로 자랐다");
        let mut seps = internal_keys(&t);
        let n = seps.len();
        seps.sort_unstable();
        seps.dedup();
        assert_eq!(
            seps.len(),
            n,
            "push-up: 올라간 가운데 키는 아래 내부 노드에 남지 않는다"
        );
        for k in 1..=20 {
            assert_eq!(t.get(k).unwrap(), Some(k));
        }
    }

    #[test]
    fn range_both_directions_match_btreemap() {
        let mut t = tree(2, 3);
        let mut model = BTreeMap::new();
        for i in 0..200u64 {
            let k = (i * 37) % 211; // 순서를 섞는다
            assert_eq!(t.insert(k, i).unwrap(), model.insert(k, i));
        }
        for (lo, hi) in [(0, 300), (5, 5), (50, 120), (120, 50), (210, 400), (1, 0)] {
            assert_eq!(
                t.range(lo, hi).unwrap(),
                model_range(&model, lo, hi),
                "range {lo}..={hi}"
            );
            let mut rev = model_range(&model, lo, hi);
            rev.reverse();
            assert_eq!(t.range_rev(lo, hi).unwrap(), rev, "range_rev {lo}..={hi}");
        }
        assert_eq!(t.pool().pinned_frames(), 0, "핀 누수 없음");
    }

    /// A2 스모크: 프레임 4개 풀에서 높이 5 이상까지 — 내보내기가 계속 일어나도 정답과 같다.
    #[test]
    fn small_pool_grows_tall_tree() {
        let mut t = tree(4, 2);
        let mut model = BTreeMap::new();
        let mut i = 0u64;
        while t.height() < 5 {
            let k = (i * 7919) % 100_003;
            assert_eq!(t.insert(k, i).unwrap(), model.insert(k, i));
            i += 1;
        }
        for (&k, &v) in &model {
            assert_eq!(t.get(k).unwrap(), Some(v));
        }
        let all: Vec<_> = model.iter().map(|(&k, &v)| (k, v)).collect();
        assert_eq!(t.range(0, u64::MAX).unwrap(), all);
        assert!(
            t.pool().stats().evictions > 0,
            "풀이 작아 내보내기가 실제로 일어났다"
        );
        assert_eq!(t.pool().pinned_frames(), 0);
    }

    #[test]
    fn reopen_from_file_sees_same_tree() {
        use crate::storage::file::FileStorage;
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("bt.db");
        let open_pool =
            || BufferPool::new(Pager::new(FileStorage::open(&path).unwrap()).unwrap(), 3);
        let (root, height) = {
            let mut t = PagedBTree::create(open_pool(), 3).unwrap();
            for k in 0..100 {
                t.insert(k, k * 2).unwrap();
            }
            t.pool().flush().unwrap();
            (t.root(), t.height())
        }; // 닫힘
        let t = PagedBTree::open(open_pool()).unwrap();
        assert_eq!(
            (t.root(), t.height()),
            (root, height),
            "루트·높이는 메타에서 복원"
        );
        let want: Vec<_> = (0..100).map(|k| (k, k * 2)).collect();
        assert_eq!(t.range(0, 99).unwrap(), want);
    }

    #[test]
    fn open_rejects_non_btree_file() {
        let pool = BufferPool::new(Pager::new(MemStorage::new()).unwrap(), 2);
        {
            // 다른 값은 멀쩡하고 magic만 다르다(예: Hash 인덱스 파일) — magic 검사만이 잡는다.
            let (_, mut page) = pool.new_page().unwrap();
            MetaPage::init(&mut page, 1, 4).magic = u64::from_le_bytes(*b"IDXHASH_");
        }
        let err = PagedBTree::open(pool).err().expect("메타가 아니면 거부");
        assert!(matches!(err, PoolError::Io(e) if e.kind() == std::io::ErrorKind::InvalidData));
    }

    #[test]
    fn usable_through_index_trait() {
        use crate::index::Index;
        let mut idx: Box<dyn Index> = Box::new(tree(2, 4));
        for k in [3, 1, 2] {
            idx.insert(k, k * 10);
        }
        assert_eq!(idx.insert(2, 99), Some(20));
        assert_eq!(idx.range_rev(1, 3), vec![(3, 30), (2, 99), (1, 10)]);
    }

    #[test]
    #[should_panic(expected = "페이지 용량")]
    fn max_keys_must_leave_one_slot_for_overflow() {
        tree(2, LEAF_CAPACITY);
    }
}

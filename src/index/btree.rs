//! B+tree — 노드를 배열(`Vec<Node>`)에 두고 배열 번호(`NodeId`)로 연결한다(T5).
//! 2단계에서 `NodeId`가 그대로 페이지 번호(`PageId`)가 된다.

/// 노드의 배열 번호.
pub type NodeId = usize;

/// 노드가 갈라졌을 때 부모에게 알리는 것: (부모에 넣을 구분자, 새로 생긴 오른쪽 노드).
type Split = (u64, NodeId);

enum Node {
    /// 이정표 노드: 키 n개 + 자식 n+1개. 구분자 규칙 = 왼쪽 < 구분자 ≤ 오른쪽.
    Internal {
        keys: Vec<u64>,
        children: Vec<NodeId>,
    },
    /// 데이터 노드: `keys[i]`의 값 = `values[i]`. 옆 리프와 양방향 연결(T7).
    Leaf {
        keys: Vec<u64>,
        values: Vec<u64>,
        prev: Option<NodeId>,
        next: Option<NodeId>,
    },
}

pub struct BTree {
    nodes: Vec<Node>,
    root: NodeId,
    max_keys: usize,
}

impl BTree {
    /// `max_keys` = 노드 하나에 들어가는 최대 키 수(fan-out, T6).
    /// 1이면 내부 분할 때 오른쪽 노드가 키 없이 남으므로 2 이상만 받는다.
    pub fn new(max_keys: usize) -> Self {
        assert!(max_keys >= 2, "max_keys는 2 이상이어야 한다");
        let empty_leaf = Node::Leaf {
            keys: Vec::new(),
            values: Vec::new(),
            prev: None,
            next: None,
        };
        BTree {
            nodes: vec![empty_leaf],
            root: 0,
            max_keys,
        }
    }

    pub fn get(&self, key: u64) -> Option<u64> {
        let Node::Leaf { keys, values, .. } = &self.nodes[self.find_leaf(key)] else {
            unreachable!("find_leaf는 리프만 돌려준다");
        };
        keys.binary_search(&key).ok().map(|i| values[i])
    }

    /// 같은 키가 있으면 값을 바꾸고 이전 값을 돌려준다(upsert).
    pub fn insert(&mut self, key: u64, value: u64) -> Option<u64> {
        let (old, split) = self.insert_at(self.root, key, value);
        if let Some((sep, right)) = split {
            // 루트가 갈라졌다 → 새 루트를 맨 위에 올린다. 깊이는 여기서만 1 늘어난다.
            let new_root = self.nodes.len();
            self.nodes.push(Node::Internal {
                keys: vec![sep],
                children: vec![self.root, right],
            });
            self.root = new_root;
        }
        old
    }

    /// `id` 아래에 넣는다. 반환 = (이전 값, `id`가 갈라졌으면 그 `Split`).
    fn insert_at(&mut self, id: NodeId, key: u64, value: u64) -> (Option<u64>, Option<Split>) {
        match &mut self.nodes[id] {
            Node::Leaf { keys, values, .. } => {
                match keys.binary_search(&key) {
                    Ok(i) => return (Some(std::mem::replace(&mut values[i], value)), None),
                    Err(i) => {
                        keys.insert(i, key);
                        values.insert(i, value);
                    }
                }
                if keys.len() <= self.max_keys {
                    return (None, None);
                }
                (None, Some(self.split_leaf(id)))
            }
            Node::Internal { keys, children } => {
                let i = keys.partition_point(|&sep| sep <= key);
                let child = children[i];
                let (old, split) = self.insert_at(child, key, value);
                if let Some((sep, right)) = split {
                    // 갈라진 자식 바로 오른쪽 칸에 구분자와 새 자식을 끼운다.
                    let Node::Internal { keys, children } = &mut self.nodes[id] else {
                        unreachable!("id는 위에서 Internal이었다");
                    };
                    keys.insert(i, sep);
                    children.insert(i + 1, right);
                    if keys.len() > self.max_keys {
                        return (old, Some(self.split_internal(id)));
                    }
                }
                (old, None)
            }
        }
    }

    /// 넘친 내부 노드를 가운데 키 기준으로 나눈다. 가운데 키는 위로 "이동"한다(push-up — 아래에 남지 않는다).
    /// keys [k0 .. k(mid-1)] · k(mid) · [k(mid+1) ..] → 왼쪽(id) · 부모로 · 오른쪽(새 노드)
    /// 자식은 mid+1개가 왼쪽에 남고 나머지가 오른쪽으로 간다.
    fn split_internal(&mut self, id: NodeId) -> Split {
        let new_id = self.nodes.len();
        let Node::Internal { keys, children } = &mut self.nodes[id] else {
            unreachable!("split_internal은 내부 노드에만 쓴다");
        };
        let mid = keys.len() / 2;
        let right_keys = keys.split_off(mid + 1);
        let sep = keys.pop().expect("mid < len이므로 가운데 키가 있다");
        let right_children = children.split_off(mid + 1);
        self.nodes.push(Node::Internal {
            keys: right_keys,
            children: right_children,
        });
        (sep, new_id)
    }

    /// 넘친 리프 A를 반으로 나눠 오른쪽 절반을 새 리프 A'로 옮긴다.
    /// 구분자 = A'의 첫 키를 복사(copy-up — 리프에도 남는다).
    /// 연결: A ⇄ C  →  A ⇄ A' ⇄ C (C의 prev까지 고친다).
    fn split_leaf(&mut self, id: NodeId) -> Split {
        let new_id = self.nodes.len();
        let Node::Leaf {
            keys, values, next, ..
        } = &mut self.nodes[id]
        else {
            unreachable!("split_leaf는 리프에만 쓴다");
        };
        let mid = keys.len() / 2;
        let right_keys = keys.split_off(mid);
        let right_values = values.split_off(mid);
        let sep = right_keys[0];
        let old_next = next.replace(new_id); // A.next = A'
        self.nodes.push(Node::Leaf {
            keys: right_keys,
            values: right_values,
            prev: Some(id), // A'.prev = A
            next: old_next, // A'.next = C
        });
        if let Some(c) = old_next {
            let Node::Leaf { prev, .. } = &mut self.nodes[c] else {
                unreachable!("리프의 next는 리프다");
            };
            *prev = Some(new_id); // C.prev = A'
        }
        (sep, new_id)
    }

    /// 루트에서 구분자를 따라 내려가 `key`가 있어야 할 리프의 번호를 돌려준다.
    fn find_leaf(&self, key: u64) -> NodeId {
        let mut id = self.root;
        loop {
            match &self.nodes[id] {
                Node::Internal { keys, children } => {
                    // key 이하인 구분자 개수 = 내려갈 자식 칸 (key == 구분자면 오른쪽으로)
                    let i = keys.partition_point(|&sep| sep <= key);
                    id = children[i];
                }
                Node::Leaf { .. } => return id,
            }
        }
    }

    /// `lo <= key <= hi`를 오름차순으로. lo가 있어야 할 리프에서 시작해 next 링크만 따라간다(내부 노드 재방문 없음).
    pub fn range(&self, lo: u64, hi: u64) -> Vec<(u64, u64)> {
        let mut out = Vec::new();
        if lo > hi {
            return out;
        }
        let mut cur = Some(self.find_leaf(lo));
        while let Some(id) = cur {
            let Node::Leaf {
                keys, values, next, ..
            } = &self.nodes[id]
            else {
                unreachable!("리프의 next는 리프다");
            };
            let start = keys.partition_point(|&k| k < lo); // 첫 리프에서만 의미 있음(이후 리프는 전부 ≥ lo)
            for (&k, &v) in keys[start..].iter().zip(&values[start..]) {
                if k > hi {
                    return out;
                }
                out.push((k, v));
            }
            cur = *next;
        }
        out
    }

    /// `range`와 같은 범위를 내림차순으로. hi가 있어야 할 리프에서 시작해 prev 링크만 따라간다.
    pub fn range_rev(&self, lo: u64, hi: u64) -> Vec<(u64, u64)> {
        let mut out = Vec::new();
        if lo > hi {
            return out;
        }
        let mut cur = Some(self.find_leaf(hi));
        while let Some(id) = cur {
            let Node::Leaf {
                keys, values, prev, ..
            } = &self.nodes[id]
            else {
                unreachable!("리프의 prev는 리프다");
            };
            let end = keys.partition_point(|&k| k <= hi); // 첫 리프에서만 의미 있음(이전 리프는 전부 ≤ hi)
            for (&k, &v) in keys[..end].iter().zip(&values[..end]).rev() {
                if k < lo {
                    return out;
                }
                out.push((k, v));
            }
            cur = *prev;
        }
        out
    }
}

impl super::Index for BTree {
    fn insert(&mut self, key: u64, value: u64) -> Option<u64> {
        BTree::insert(self, key, value)
    }

    fn get(&self, key: u64) -> Option<u64> {
        BTree::get(self, key)
    }

    fn range(&self, lo: u64, hi: u64) -> Vec<(u64, u64)> {
        BTree::range(self, lo, hi)
    }

    fn range_rev(&self, lo: u64, hi: u64) -> Vec<(u64, u64)> {
        BTree::range_rev(self, lo, hi)
    }
}

// ---------- 구조 관찰·검사 (T12: depth·check 공개) ----------

impl BTree {
    /// 높이 — 루트에서 가장 왼쪽 경로로 내려가 센 노드 수(리프만 있으면 1). T12 공개.
    pub fn depth(&self) -> usize {
        let mut id = self.root;
        let mut h = 1;
        while let Node::Internal { children, .. } = &self.nodes[id] {
            id = children[0];
            h += 1;
        }
        h
    }

    /// I2 구조 불변식 전체 검사(T12 — 테스트와 3단계 실험이 같이 쓴다). 어긋나면 이유와 함께 panic.
    pub fn check(&self) {
        let t = self;
        assert!(t.max_keys >= 2, "max_keys {} < 2", t.max_keys);
        assert!(
            t.root < t.nodes.len(),
            "루트 {}가 arena 밖(len {})",
            t.root,
            t.nodes.len()
        );
        let mut w = Walk {
            seen: vec![false; t.nodes.len()],
            leaves: Vec::new(),
        };
        walk(t, t.root, 0, None, None, &mut w);

        // 고아 노드 없음: arena의 모든 노드가 루트에서 정확히 한 번 도달.
        if let Some(orphan) = w.seen.iter().position(|s| !s) {
            panic!("노드 {orphan}가 루트에서 도달 불가(고아)");
        }

        // 모든 리프 깊이 같음.
        let d0 = w.leaves[0].1;
        for &(id, d) in &w.leaves {
            assert_eq!(d, d0, "리프 {id} 깊이 {d} != 첫 리프 깊이 {d0}");
        }

        let order: Vec<NodeId> = w.leaves.iter().map(|&(id, _)| id).collect();
        let first = order[0];
        let last = *order.last().unwrap();
        assert_eq!(
            leaf(t, first).2,
            None,
            "첫 리프 {first}의 prev가 None이 아님"
        );
        assert_eq!(
            leaf(t, last).3,
            None,
            "마지막 리프 {last}의 next가 None이 아님"
        );

        // a.next == Some(b) ⇔ b.prev == Some(a)
        for &a in &order {
            let (_, _, prev, next) = leaf(t, a);
            if let Some(b) = next {
                assert_eq!(
                    leaf(t, b).2,
                    Some(a),
                    "리프 {a}.next = {b}인데 {b}.prev != {a}"
                );
            }
            if let Some(b) = prev {
                assert_eq!(
                    leaf(t, b).3,
                    Some(a),
                    "리프 {a}.prev = {b}인데 {b}.next != {a}"
                );
            }
        }

        // next 사슬 = 중위 리프 순서, 키 전체가 엄격 오름차순.
        let limit = t.nodes.len();
        let mut fwd = Vec::new();
        let mut cur = Some(first);
        while let Some(id) = cur {
            assert!(fwd.len() < limit, "next 사슬이 끝나지 않음(순환)");
            fwd.push(id);
            cur = leaf(t, id).3;
        }
        assert_eq!(fwd, order, "next 사슬 순서가 트리 중위 리프 순서와 다름");
        let all: Vec<u64> = fwd
            .iter()
            .flat_map(|&id| leaf(t, id).0.iter().copied())
            .collect();
        assert!(
            all.windows(2).all(|p| p[0] < p[1]),
            "next 사슬로 읽은 키가 엄격 오름차순이 아님"
        );

        // prev 사슬 = 중위 리프 순서의 역순.
        let mut bwd = Vec::new();
        let mut cur = Some(last);
        while let Some(id) = cur {
            assert!(bwd.len() < limit, "prev 사슬이 끝나지 않음(순환)");
            bwd.push(id);
            cur = leaf(t, id).2;
        }
        bwd.reverse();
        assert_eq!(bwd, order, "prev 사슬 순서가 중위 리프 역순과 다름");
    }
}

/// 리프 하나의 필드를 꺼낸다. 리프가 아니면 panic.
fn leaf(t: &BTree, id: NodeId) -> (&[u64], &[u64], Option<NodeId>, Option<NodeId>) {
    match &t.nodes[id] {
        Node::Leaf {
            keys,
            values,
            prev,
            next,
        } => (keys, values, *prev, *next),
        Node::Internal { .. } => panic!("노드 {id}: 리프여야 하는데 내부 노드"),
    }
}

struct Walk {
    seen: Vec<bool>,
    /// 중위 순서로 만난 (리프 번호, 깊이).
    leaves: Vec<(NodeId, usize)>,
}

/// 서브트리를 검사하고 서브트리의 최소 키를 돌려준다(빈 루트 리프면 None).
/// `lo <= k < hi`가 이 서브트리 모든 키의 허용 범위.
fn walk(
    t: &BTree,
    id: NodeId,
    depth: usize,
    lo: Option<u64>,
    hi: Option<u64>,
    w: &mut Walk,
) -> Option<u64> {
    let m = t.max_keys;
    assert!(
        id < t.nodes.len(),
        "노드 번호 {id}가 arena 밖(len {})",
        t.nodes.len()
    );
    assert!(!w.seen[id], "노드 {id}가 두 번 이상 참조됨(공유·순환)");
    w.seen[id] = true;
    let is_root = id == t.root;

    let keys: &[u64] = match &t.nodes[id] {
        Node::Leaf { keys, .. } | Node::Internal { keys, .. } => keys,
    };
    assert!(
        keys.len() <= m,
        "노드 {id}: 키 {}개 > max_keys {m}",
        keys.len()
    );
    assert!(
        keys.windows(2).all(|p| p[0] < p[1]),
        "노드 {id}: 키가 엄격 오름차순이 아님 {keys:?}"
    );
    for &k in keys {
        assert!(
            lo.is_none_or(|l| l <= k) && hi.is_none_or(|h| k < h),
            "노드 {id}: 키 {k}가 구분자 범위 [{lo:?}, {hi:?}) 밖"
        );
    }

    match &t.nodes[id] {
        Node::Leaf { values, .. } => {
            assert_eq!(
                values.len(),
                keys.len(),
                "리프 {id}: values 길이 != keys 길이"
            );
            // 리프 분할: max+1개를 반으로 → 작은 쪽 ⌊(max+1)/2⌋ = ⌈max/2⌉.
            if !is_root {
                assert!(
                    keys.len() >= m.div_ceil(2),
                    "리프 {id}: 키 {}개 < 하한 ⌈{m}/2⌉ ({keys:?})",
                    keys.len()
                );
            }
            w.leaves.push((id, depth));
            keys.first().copied()
        }
        Node::Internal { children, .. } => {
            assert_eq!(
                children.len(),
                keys.len() + 1,
                "내부 {id}: children {}개 != keys {}개 + 1",
                children.len(),
                keys.len()
            );
            if is_root {
                assert!(!keys.is_empty(), "루트 내부 노드 {id}에 키가 없음");
            } else {
                // 내부 분할: max+1개 중 가운데 1개를 올리고 남은 max개를 반으로 → ⌊max/2⌋.
                assert!(
                    keys.len() >= m / 2,
                    "내부 {id}: 키 {}개 < 하한 ⌊{m}/2⌋ ({keys:?})",
                    keys.len()
                );
            }
            let mut first_min = None;
            for (i, &c) in children.iter().enumerate() {
                let clo = if i == 0 { lo } else { Some(keys[i - 1]) };
                let chi = if i < keys.len() { Some(keys[i]) } else { hi };
                let min = walk(t, c, depth + 1, clo, chi, w);
                assert!(min.is_some(), "내부 {id}의 자식 {c} 서브트리가 비어 있음");
                if i == 0 {
                    first_min = min;
                } else {
                    // copy-up/push-up이면 구분자 = 오른쪽 서브트리의 최소 키(삭제 없음).
                    assert_eq!(
                        min,
                        Some(keys[i - 1]),
                        "내부 {id}: 구분자 {}가 오른쪽 서브트리 최소 키와 다름",
                        keys[i - 1]
                    );
                }
            }
            first_min
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;
    use std::collections::BTreeMap;

    // ---------- 테스트 보조 ----------

    /// 내부 노드의 (keys, children). 내부 노드가 아니면 panic.
    fn internal(t: &BTree, id: NodeId) -> (&[u64], &[NodeId]) {
        match &t.nodes[id] {
            Node::Internal { keys, children } => (keys, children),
            Node::Leaf { .. } => panic!("노드 {id}: 내부 노드여야 하는데 리프"),
        }
    }

    /// 중위 순서(왼→오) 리프의 키 목록.
    fn leaf_keys_in_order(t: &BTree) -> Vec<Vec<u64>> {
        fn go(t: &BTree, id: NodeId, out: &mut Vec<Vec<u64>>) {
            match &t.nodes[id] {
                Node::Leaf { keys, .. } => out.push(keys.clone()),
                Node::Internal { children, .. } => {
                    for &c in children {
                        go(t, c, out);
                    }
                }
            }
        }
        let mut out = Vec::new();
        go(t, t.root, &mut out);
        out
    }

    /// 트리에 들어 있는 (키, 값) 전체 — 정답지와 비교용.
    fn entries(t: &BTree) -> Vec<(u64, u64)> {
        t.range(0, u64::MAX)
    }

    // ---------- 단위 테스트 ----------

    #[test]
    fn new_tree_is_empty() {
        let t = BTree::new(4);
        t.check();
        assert_eq!(t.depth(), 1);
        assert_eq!(t.get(0), None);
        assert_eq!(t.get(u64::MAX), None);
        assert!(t.range(0, u64::MAX).is_empty());
        assert!(t.range_rev(0, u64::MAX).is_empty());
    }

    #[test]
    #[should_panic]
    fn new_with_max_keys_1_panics() {
        let _ = BTree::new(1);
    }

    #[test]
    fn min_max_keys_2_works() {
        let mut t = BTree::new(2);
        for k in 0..50 {
            assert_eq!(t.insert(k, k * 10), None);
            t.check();
        }
        assert_eq!(
            entries(&t),
            (0..50).map(|k| (k, k * 10)).collect::<Vec<_>>()
        );
    }

    #[test]
    fn upsert_returns_old_value_and_replaces() {
        let mut t = BTree::new(3);
        assert_eq!(t.insert(5, 50), None);
        assert_eq!(t.insert(5, 51), Some(50));
        assert_eq!(t.get(5), Some(51));
        assert_eq!(t.insert(5, 52), Some(51));
        assert_eq!(t.get(5), Some(52));
        assert_eq!(entries(&t), vec![(5, 52)]);
        t.check();
    }

    #[test]
    fn upsert_into_full_leaf_does_not_split() {
        let mut t = BTree::new(3);
        for k in [1, 2, 3] {
            t.insert(k, k);
        }
        let before = t.nodes.len();
        assert_eq!(t.insert(2, 20), Some(2));
        assert_eq!(t.nodes.len(), before, "upsert가 노드를 새로 만듦");
        assert_eq!(t.depth(), 1);
        assert_eq!(entries(&t), vec![(1, 1), (2, 20), (3, 3)]);
        t.check();
    }

    #[test]
    fn upsert_after_splits_keeps_structure() {
        let mut t = BTree::new(3);
        for k in 1..=40 {
            t.insert(k, k);
        }
        let before = t.nodes.len();
        for k in 1..=40 {
            assert_eq!(t.insert(k, k + 1000), Some(k));
        }
        assert_eq!(t.nodes.len(), before);
        t.check();
        for k in 1..=40 {
            assert_eq!(t.get(k), Some(k + 1000));
        }
    }

    #[test]
    fn no_split_until_overflow() {
        let mut t = BTree::new(3);
        for k in [10, 20, 30] {
            t.insert(k, k);
        }
        t.check();
        assert_eq!(t.depth(), 1);
        assert_eq!(leaf(&t, t.root).0, &[10, 20, 30]);
    }

    #[test]
    fn first_leaf_split_copies_up_separator() {
        let mut t = BTree::new(3);
        for k in [10, 20, 30, 40] {
            t.insert(k, k + 1);
        }
        t.check();
        assert_eq!(t.depth(), 2);
        let (keys, children) = internal(&t, t.root);
        assert_eq!(keys, &[30]);
        assert_eq!(children.len(), 2);
        let (lk, lv, lp, ln) = leaf(&t, children[0]);
        let (rk, rv, rp, rn) = leaf(&t, children[1]);
        assert_eq!((lk, lv), (&[10, 20][..], &[11, 21][..]));
        // 구분자 30은 오른쪽 리프에 그대로 남는다(copy-up).
        assert_eq!((rk, rv), (&[30, 40][..], &[31, 41][..]));
        assert_eq!((lp, ln), (None, Some(children[1])));
        assert_eq!((rp, rn), (Some(children[0]), None));
    }

    #[test]
    fn internal_split_pushes_up_middle_and_grows_root() {
        let mut t = BTree::new(3);
        for k in (10..=100).step_by(10) {
            t.insert(k, k);
        }
        t.check();
        assert_eq!(t.depth(), 3);
        let (rk, rc) = internal(&t, t.root);
        assert_eq!(rk, &[70]);
        let (lk, _) = internal(&t, rc[0]);
        let (rrk, _) = internal(&t, rc[1]);
        // 70은 내부 노드에서 빠져 루트로만 올라간다(push-up).
        assert_eq!(lk, &[30, 50]);
        assert_eq!(rrk, &[90]);
        assert_eq!(
            leaf_keys_in_order(&t),
            vec![
                vec![10, 20],
                vec![30, 40],
                vec![50, 60],
                vec![70, 80],
                vec![90, 100]
            ]
        );
    }

    #[test]
    fn sequential_and_reverse_insertion() {
        for m in 2..=6 {
            for reverse in [false, true] {
                let mut t = BTree::new(m);
                let keys: Vec<u64> = if reverse {
                    (1..=500).rev().collect()
                } else {
                    (1..=500).collect()
                };
                for &k in &keys {
                    assert_eq!(t.insert(k, !k), None);
                }
                t.check();
                assert!(t.depth() >= 3, "m={m}: 500개인데 높이 {}", t.depth());
                let want: Vec<(u64, u64)> = (1..=500).map(|k| (k, !k)).collect();
                assert_eq!(entries(&t), want, "m={m} reverse={reverse}");
                let mut rev = want.clone();
                rev.reverse();
                assert_eq!(t.range_rev(0, u64::MAX), rev);
                assert_eq!(t.get(0), None);
                assert_eq!(t.get(501), None);
            }
        }
    }

    #[test]
    fn range_boundaries() {
        let mut t = BTree::new(3);
        for k in (10..=100).step_by(10) {
            t.insert(k, k * 2);
        }
        let r = |lo, hi| {
            t.range(lo, hi)
                .into_iter()
                .map(|(k, _)| k)
                .collect::<Vec<_>>()
        };
        let rr = |lo, hi| {
            t.range_rev(lo, hi)
                .into_iter()
                .map(|(k, _)| k)
                .collect::<Vec<_>>()
        };
        // 양끝 포함(T14), 리프 경계를 가로지름.
        assert_eq!(r(30, 70), vec![30, 40, 50, 60, 70]);
        assert_eq!(rr(30, 70), vec![70, 60, 50, 40, 30]);
        // 경계가 키 사이에 떨어짐.
        assert_eq!(r(25, 65), vec![30, 40, 50, 60]);
        assert_eq!(rr(25, 65), vec![60, 50, 40, 30]);
        // lo == hi
        assert_eq!(r(50, 50), vec![50]);
        assert_eq!(rr(50, 50), vec![50]);
        assert!(r(55, 55).is_empty());
        assert!(rr(55, 55).is_empty());
        // lo > hi → 빈 결과
        assert!(r(70, 30).is_empty());
        assert!(rr(70, 30).is_empty());
        assert!(r(u64::MAX, 0).is_empty());
        assert!(rr(u64::MAX, 0).is_empty());
        // 모든 키 밖
        assert!(r(0, 9).is_empty());
        assert!(rr(0, 9).is_empty());
        assert!(r(101, u64::MAX).is_empty());
        assert!(rr(101, u64::MAX).is_empty());
        // 한쪽 끝만 걸침
        assert_eq!(r(0, 10), vec![10]);
        assert_eq!(r(100, u64::MAX), vec![100]);
        assert_eq!(rr(100, u64::MAX), vec![100]);
        // 전체
        let all: Vec<u64> = (10..=100).step_by(10).collect();
        assert_eq!(r(0, u64::MAX), all);
        let mut all_rev = all.clone();
        all_rev.reverse();
        assert_eq!(rr(0, u64::MAX), all_rev);
        // 값도 같이 나온다.
        assert_eq!(t.range(40, 40), vec![(40, 80)]);
    }

    #[test]
    fn keys_at_u64_extremes() {
        let mut t = BTree::new(2);
        let ks = [u64::MAX, 0, u64::MAX - 1, 1, u64::MAX - 2, 2, u64::MAX / 2];
        for &k in &ks {
            assert_eq!(t.insert(k, k ^ 1), None);
            t.check();
        }
        assert_eq!(t.get(u64::MAX), Some(u64::MAX ^ 1));
        assert_eq!(t.get(0), Some(1));
        assert_eq!(t.range(u64::MAX, u64::MAX), vec![(u64::MAX, u64::MAX ^ 1)]);
        assert_eq!(
            t.range_rev(u64::MAX, u64::MAX),
            vec![(u64::MAX, u64::MAX ^ 1)]
        );
        assert_eq!(
            t.range(u64::MAX - 1, u64::MAX),
            vec![(u64::MAX - 1, (u64::MAX - 1) ^ 1), (u64::MAX, u64::MAX ^ 1)]
        );
        assert_eq!(t.range(0, 0), vec![(0, 1)]);
        let mut sorted = ks.to_vec();
        sorted.sort_unstable();
        let got: Vec<u64> = t.range(0, u64::MAX).into_iter().map(|(k, _)| k).collect();
        assert_eq!(got, sorted);
        assert_eq!(t.insert(u64::MAX, 7), Some(u64::MAX ^ 1));
        t.check();
    }

    #[test]
    fn works_through_dyn_index() {
        let mut t = BTree::new(3);
        let idx: &mut dyn crate::index::Index = &mut t;
        for k in 0..30 {
            assert_eq!(idx.insert(k, k + 1), None);
        }
        assert_eq!(idx.insert(7, 0), Some(8));
        assert_eq!(idx.get(7), Some(0));
        assert_eq!(idx.get(30), None);
        assert_eq!(idx.range(5, 8), vec![(5, 6), (6, 7), (7, 0), (8, 9)]);
        assert_eq!(idx.range_rev(5, 8), vec![(8, 9), (7, 0), (6, 7), (5, 6)]);
        assert!(idx.range(8, 5).is_empty());
        t.check();
    }

    // ---------- 속성 테스트 (I1·I2·I4, 정답지 BTreeMap) ----------

    /// 좁은 키 공간(중복·upsert 유도) 위주 + u64 끝 근처 + 전체 u64.
    fn key() -> impl Strategy<Value = u64> {
        prop_oneof![
            6 => 0u64..200,
            1 => (u64::MAX - 20)..=u64::MAX,
            1 => any::<u64>(),
        ]
    }

    fn ops(max_len: usize) -> impl Strategy<Value = Vec<(u64, u64)>> {
        prop::collection::vec((key(), any::<u64>()), 0..max_len)
    }

    /// `[lo, hi]` 정답(lo > hi면 빈 결과 — BTreeMap::range는 이때 panic하므로 막는다).
    fn model_range(map: &BTreeMap<u64, u64>, lo: u64, hi: u64) -> Vec<(u64, u64)> {
        if lo > hi {
            return Vec::new();
        }
        map.range(lo..=hi).map(|(&k, &v)| (k, v)).collect()
    }

    fn model_range_rev(map: &BTreeMap<u64, u64>, lo: u64, hi: u64) -> Vec<(u64, u64)> {
        if lo > hi {
            return Vec::new();
        }
        map.range(lo..=hi).rev().map(|(&k, &v)| (k, v)).collect()
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(256))]

        #[test]
        fn matches_btreemap(
            max_keys in 2usize..=8,
            ops in ops(400),
            queries in prop::collection::vec((key(), key()), 0..32),
        ) {
            let mut t = BTree::new(max_keys);
            let mut map = BTreeMap::new();
            for &(k, v) in &ops {
                prop_assert_eq!(t.insert(k, v), map.insert(k, v), "insert({}, {})", k, v);
            }
            t.check();

            let want: Vec<(u64, u64)> = map.iter().map(|(&k, &v)| (k, v)).collect();
            prop_assert_eq!(t.range(0, u64::MAX), want);

            for &(k, _) in &ops {
                prop_assert_eq!(t.get(k), map.get(&k).copied());
            }
            for &(lo, hi) in &queries {
                prop_assert_eq!(t.get(lo), map.get(&lo).copied());
                prop_assert_eq!(t.range(lo, hi), model_range(&map, lo, hi), "range({}, {})", lo, hi);
                prop_assert_eq!(
                    t.range_rev(lo, hi),
                    model_range_rev(&map, lo, hi),
                    "range_rev({}, {})", lo, hi
                );
            }

            // 같은 대조를 trait 객체로 한 번 더.
            let idx: &dyn crate::index::Index = &t;
            for &(lo, hi) in &queries {
                prop_assert_eq!(idx.get(hi), map.get(&hi).copied());
                prop_assert_eq!(idx.range(lo, hi), model_range(&map, lo, hi));
                prop_assert_eq!(idx.range_rev(lo, hi), model_range_rev(&map, lo, hi));
            }
        }

        /// 삽입 한 번마다 구조 검사 — 분할 직후의 중간 상태도 I2를 지키는지.
        #[test]
        fn structure_holds_after_every_insert(
            max_keys in 2usize..=5,
            ops in ops(120),
        ) {
            let mut t = BTree::new(max_keys);
            let mut map = BTreeMap::new();
            for &(k, v) in &ops {
                prop_assert_eq!(t.insert(k, v), map.insert(k, v));
                t.check();
            }
            let want: Vec<(u64, u64)> = map.iter().rev().map(|(&k, &v)| (k, v)).collect();
            prop_assert_eq!(t.range_rev(0, u64::MAX), want);
        }
    }
}

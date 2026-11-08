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
                    // ponytail: 내부 노드 넘침(keys.len() > max_keys)은 아직 그대로 둔다 — diff 04에서 push-up 분할.
                }
                (old, None)
            }
        }
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
}

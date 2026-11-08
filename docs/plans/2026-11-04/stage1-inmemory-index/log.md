# log — stage1-inmemory-index (lazy)

## 타임라인

| 시각 | 사건 | 결과/결정 |
|------|------|----------|
| 2026-11-04 | 리서치(내부·외부) + 트레이드오프 T1~T13 사용자 선택 | 명세 §9·§10 |
| 2026-11-08 | 계획 이해 게이트 — Q1(구조)·Q2(2단계 대비)·Q3(검증) 사용자 주관식 → 독립 워커(Opus) 판정 | 1/2회차 PASS. 보완 권고: 내부 노드도 구분자 키 보유, copy-up(리프에 남음) vs push-up(빠짐) |
| 2026-11-08 | 명세 승인(SPEC=1) · lazy · 브랜치 feat/stage1-inmemory-index | log 생성 → 게이트 재기록 |
| 2026-11-08 | diff 01a 게이트 1/2 — 판정 워커(Opus) | fail: get·range·range_rev 약속 누락, trait 구현 주체·정답지 비교, const가 무엇을 언제 막는지 누락 → 재질문 |
| 2026-11-08 | diff 01a 게이트 2/2 — 판정 워커(Opus) | fail → 지적 후 통과 처리: range_rev(같은 범위 내림차순) 누락, const = 컴파일 시점에 dyn 비호환 변경 감지(3단계 Box<dyn Index>) 설명 부족 |
| 2026-11-08 | diff 01b lib.rs 연결 → cargo test·clippy -D warnings·fmt --check 통과. 변이: trait에 제네릭 메서드 추가 → E0038 "not dyn compatible"로 컴파일 실패 확인. 복원 실수: src 미추적이라 git checkout이 실패해 변이가 남음 → sed로 그 줄 삭제·mod.rs가 01a 스니펫과 동일함 재확인 | 01b 게이트 1/2 pass (Opus) |
| 2026-11-08 | diff 01c Cargo.toml dev-dep proptest = "1.12.0" → A1 실증: rustc 1.92.0에서 proptest v1.12.0 컴파일, cargo test exit 0, clippy -D warnings 통과. 런타임 의존성 0(cargo tree -e normal), dev 의존성 proptest 1개 | 01c 게이트 1/2 pass (Opus) |
| 2026-11-08 | diff 01 커밋(6d67b5f 코드 · d1841c6 docs) → diff 02a src/index/btree.rs(노드 구조체 + get). mod 선언은 02b(쓰기 1회=게이트 1회). 검증: rustc 단독 컴파일 -D warnings 통과 + scratch 스모크(손으로 만든 [30]→[10,20]|[30,40] 트리에서 get 8케이스·find_leaf(29)=왼쪽·(30)=오른쪽) 통과 | 02a 게이트 1/2 fail (Opus): 리프 binary_search·없으면 None 누락, "같으면 오른쪽"의 이유(구분자 규칙·구분자 출처) 누락 → 재질문 · 2/2 pass (Opus) |
| 2026-11-08 | diff 02b mod.rs에 `pub mod btree;` → cargo test exit 0 · clippy -D warnings 통과(expect 충족) · **fmt --check 실패**: btree.rs의 `#[expect(dead_code, reason = …)]` 한 줄이 길어 rustfmt가 여러 줄로 바꾸려 함 → btree.rs를 다시 쓰는 diff 03에서 함께 정리(Bash `cargo fmt`로 게이트 우회하지 않음). 커밋은 fmt 통과 후 | 02b 게이트 1/2 pass (Opus) |
| 2026-11-08 | diff 03 btree.rs insert·insert_at·split_leaf + 루트 분할(새 루트)을 04에서 03으로 당김(첫 리프 분할부터 필요) — 04에는 내부 분할만 남음. scratch 스모크 4/4(순차·역순 60개 max 3, 섞인 200개 max 4·2: insert 반환·get이 BTreeMap과 일치, next 체인 오름차순·prev 체인 내림차순 전체 일치, 첫 분할 [30]→[10,20]|[30,40]). **clippy 실패**: Node의 expect(dead_code) 미충족(split_leaf에서 prev를 바인딩해 읽음으로 집계) · **fmt 실패**: 주석 앞 공백 2칸 → 03b에서 수정 | 03 게이트 대기 |
| 2026-11-09 | diff 03 게이트 1차 답변 판정(Opus 워커) — 전파 경로(재귀 반환·keys/children 끼우기)만 충족, split_leaf 동작·구분자(오른쪽 첫 키 복사)·링크 재연결·루트 분할(새 루트) 누락 | fail 1/2 — 정답 비노출 재질문 |
| 2026-11-09 | diff 03 2차 시도 대신 사용자가 설명 요청 → 2차 기회 포기로 보고 fail 2/2 규칙 적용(정답 설명 후 통과 처리) — 누락 4점(반 나눔·오른쪽 첫 키 복사·링크 4개·새 루트) 설명 | 03 통과(2/2 설명) |
| 2026-11-09 | diff 03b btree.rs — Node의 `#[expect(dead_code)]` 삭제(prev 바인딩으로 미충족) · 주석 앞 공백 2→1(fmt). 사용자 요청: 커밋·push 후 다음 진행 → cargo test exit 0 · clippy -D warnings 통과 · fmt --check 통과 · scratch 스모크 4/4 재통과. 사용자 요청으로 구조·알고리즘 그림 문서 docs/learning/stage1/btree.md 작성 | 03b 게이트 대기(커밋·push 사용자 요청) |

## diff 기록 (before/after — 판정 워커 그라운드 트루스)

### diff 01a — `cargo init --lib` + `Index` trait

- T14 추가(diff 직전 사용자 선택): `range` 경계 = 양끝 포함 `[lo, hi]`.
- 훅이 파일 쓰기 1회마다 게이트를 걸어 diff 01을 01a(trait) / 01b(Cargo.toml dev-dep·lib.rs 연결)로 나눔.

`cargo init --lib --name index-lab --vcs none` 생성물 (before: 없음):

```toml
# Cargo.toml
[package]
name = "index-lab"
version = "0.1.0"
edition = "2024"

[dependencies]
```

`src/index/mod.rs` (before: 없음 → after):

```rust
pub trait Index {
    /// `key`에 `value`를 넣는다. 같은 키가 이미 있으면 값을 바꾸고 이전 값을 돌려준다(upsert).
    fn insert(&mut self, key: u64, value: u64) -> Option<u64>;

    /// `key`의 값을 돌려준다. 없으면 `None`.
    fn get(&self, key: u64) -> Option<u64>;

    /// `lo <= key <= hi`인 (키, 값)을 키 오름차순으로 돌려준다. `lo > hi`면 빈 결과.
    fn range(&self, lo: u64, hi: u64) -> Vec<(u64, u64)>;

    /// `range`와 같은 범위를 키 내림차순으로 돌려준다.
    fn range_rev(&self, lo: u64, hi: u64) -> Vec<(u64, u64)>;
}

// 3단계 러너가 `Box<dyn Index>`로 구현을 바꿔 끼운다 — trait가 dyn으로 못 쓰이게 바뀌면 여기서 컴파일이 깨진다.
const _: Option<&dyn Index> = None;
```

### diff 01b — `src/lib.rs` 모듈 연결

before (`cargo init` 템플릿):

```rust
pub fn add(left: u64, right: u64) -> u64 {
    left + right
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn it_works() {
        let result = add(2, 2);
        assert_eq!(result, 4);
    }
}
```

after:

```rust
//! index-lab — 페이지 기반 스토리지 엔진 위 인덱스 실험. 1단계: 메모리 내 B+tree · Hash.

pub mod index;
```

### diff 01c — `Cargo.toml` dev-dependency

before (`cargo init`):

```toml
[package]
name = "index-lab"
version = "0.1.0"
edition = "2024"

[dependencies]
```

after:

```toml
[package]
name = "index-lab"
version = "0.1.0"
edition = "2024"

[dev-dependencies]
proptest = "1.12.0"
```

### diff 02a — `src/index/btree.rs` 노드 구조체 + `get` (before: 없음)

```rust
//! B+tree — 노드를 배열(`Vec<Node>`)에 두고 배열 번호(`NodeId`)로 연결한다(T5).
//! 2단계에서 `NodeId`가 그대로 페이지 번호(`PageId`)가 된다.

/// 노드의 배열 번호.
pub type NodeId = usize;

#[expect(dead_code, reason = "Internal 생성·prev/next·max_keys는 diff 03~05에서 사용")]
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
    #[expect(dead_code, reason = "분할 기준 — diff 03에서 사용")]
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
```

### diff 02b — `src/index/mod.rs` 모듈 선언

before:

```rust
//! 키는 `u64`, 값은 `u64` 고정(README 범위: 키 `u64`, 값 고정 길이).

/// 키 → 값 인덱스.
```

after:

```rust
//! 키는 `u64`, 값은 `u64` 고정(README 범위: 키 `u64`, 값 고정 길이).

pub mod btree;

/// 키 → 값 인덱스.
```

### diff 03 — B+tree `insert` · 리프 분할(copy-up) · 형제 재연결 · 루트 분할

before = diff 02a 스니펫. 변경분(`diff -u`, 실파일):

```diff
@@ -4,7 +4,10 @@
 /// 노드의 배열 번호.
 pub type NodeId = usize;
 
-#[expect(dead_code, reason = "Internal 생성·prev/next·max_keys는 diff 03~05에서 사용")]
+/// 노드가 갈라졌을 때 부모에게 알리는 것: (부모에 넣을 구분자, 새로 생긴 오른쪽 노드).
+type Split = (u64, NodeId);
+
+#[expect(dead_code, reason = "Leaf.prev는 diff 05 range_rev에서 읽는다")]
 enum Node {
     /// 이정표 노드: 키 n개 + 자식 n+1개. 구분자 규칙 = 왼쪽 < 구분자 ≤ 오른쪽.
     Internal {
@@ -23,7 +26,6 @@
 pub struct BTree {
     nodes: Vec<Node>,
     root: NodeId,
-    #[expect(dead_code, reason = "분할 기준 — diff 03에서 사용")]
     max_keys: usize,
 }
 
@@ -52,6 +54,86 @@
         keys.binary_search(&key).ok().map(|i| values[i])
     }
 
+    /// 같은 키가 있으면 값을 바꾸고 이전 값을 돌려준다(upsert).
+    pub fn insert(&mut self, key: u64, value: u64) -> Option<u64> {
+        let (old, split) = self.insert_at(self.root, key, value);
+        if let Some((sep, right)) = split {
+            // 루트가 갈라졌다 → 새 루트를 맨 위에 올린다. 깊이는 여기서만 1 늘어난다.
+            let new_root = self.nodes.len();
+            self.nodes.push(Node::Internal {
+                keys: vec![sep],
+                children: vec![self.root, right],
+            });
+            self.root = new_root;
+        }
+        old
+    }
+
+    /// `id` 아래에 넣는다. 반환 = (이전 값, `id`가 갈라졌으면 그 `Split`).
+    fn insert_at(&mut self, id: NodeId, key: u64, value: u64) -> (Option<u64>, Option<Split>) {
+        match &mut self.nodes[id] {
+            Node::Leaf { keys, values, .. } => {
+                match keys.binary_search(&key) {
+                    Ok(i) => return (Some(std::mem::replace(&mut values[i], value)), None),
+                    Err(i) => {
+                        keys.insert(i, key);
+                        values.insert(i, value);
+                    }
+                }
+                if keys.len() <= self.max_keys {
+                    return (None, None);
+                }
+                (None, Some(self.split_leaf(id)))
+            }
+            Node::Internal { keys, children } => {
+                let i = keys.partition_point(|&sep| sep <= key);
+                let child = children[i];
+                let (old, split) = self.insert_at(child, key, value);
+                if let Some((sep, right)) = split {
+                    // 갈라진 자식 바로 오른쪽 칸에 구분자와 새 자식을 끼운다.
+                    let Node::Internal { keys, children } = &mut self.nodes[id] else {
+                        unreachable!("id는 위에서 Internal이었다");
+                    };
+                    keys.insert(i, sep);
+                    children.insert(i + 1, right);
+                    // ponytail: 내부 노드 넘침(keys.len() > max_keys)은 아직 그대로 둔다 — diff 04에서 push-up 분할.
+                }
+                (old, None)
+            }
+        }
+    }
+
+    /// 넘친 리프 A를 반으로 나눠 오른쪽 절반을 새 리프 A'로 옮긴다.
+    /// 구분자 = A'의 첫 키를 복사(copy-up — 리프에도 남는다).
+    /// 연결: A ⇄ C  →  A ⇄ A' ⇄ C (C의 prev까지 고친다).
+    fn split_leaf(&mut self, id: NodeId) -> Split {
+        let new_id = self.nodes.len();
+        let Node::Leaf {
+            keys, values, next, ..
+        } = &mut self.nodes[id]
+        else {
+            unreachable!("split_leaf는 리프에만 쓴다");
+        };
+        let mid = keys.len() / 2;
+        let right_keys = keys.split_off(mid);
+        let right_values = values.split_off(mid);
+        let sep = right_keys[0];
+        let old_next = next.replace(new_id); // A.next = A'
+        self.nodes.push(Node::Leaf {
+            keys: right_keys,
+            values: right_values,
+            prev: Some(id),  // A'.prev = A
+            next: old_next, // A'.next = C
+        });
+        if let Some(c) = old_next {
+            let Node::Leaf { prev, .. } = &mut self.nodes[c] else {
+                unreachable!("리프의 next는 리프다");
+            };
+            *prev = Some(new_id); // C.prev = A'
+        }
+        (sep, new_id)
+    }
+
     /// 루트에서 구분자를 따라 내려가 `key`가 있어야 할 리프의 번호를 돌려준다.
     fn find_leaf(&self, key: u64) -> NodeId {
         let mut id = self.root;
```

### diff 03b — clippy(미충족 expect)·fmt 수정

```diff
-#[expect(dead_code, reason = "Leaf.prev는 diff 05 range_rev에서 읽는다")]
 enum Node {
```

```diff
-            prev: Some(id),  // A'.prev = A
+            prev: Some(id), // A'.prev = A
```

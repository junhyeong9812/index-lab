# B+tree — 구조 · 알고리즘 그림 정리 (1단계, diff 05 기준)

> 코드: `src/index/btree.rs` · 계약: `src/index/mod.rs`의 `Index` trait · 기록: `docs/plans/2026-11-04/stage1-inmemory-index/log.md`
> 질문·답변 정리와 오답 노트: [btree-qa.md](btree-qa.md)
> ✅ = 지금 코드에 있음 · ⏳ = 다음 diff에서 들어옴

---

## 1. 아키텍처 — 파일과 모듈

```text
src/lib.rs                 pub mod index;
 └ src/index/mod.rs         pub trait Index { insert · get · range · range_rev }   ← 약속(계약)
    ├ src/index/btree.rs    pub struct BTree   ✅ (impl Index ✅ diff 05)
    └ src/index/hash.rs     ⏳ diff 07~09 (Extendible Hash)

정답지: std::collections::BTreeMap<u64, u64> — 모든 결과를 이것과 비교 (⏳ diff 06 proptest)
```

- `mod` 선언이 모듈 트리를 만든다. 선언이 없으면 파일은 컴파일 대상조차 아니다.
- `const _: Option<&dyn Index> = None;` — 3단계 러너가 `Box<dyn Index>`로 구현을 바꿔 끼울 수 있게, trait가 dyn으로 못 쓰이게 바뀌면 컴파일을 깨뜨리는 안전장치.

---

## 2. 메모리 구조 — 배열(arena) + 배열 번호

노드끼리 포인터(`Box`)로 잇지 않고, **모든 노드를 `Vec<Node>` 하나에 넣고 번호(`NodeId = usize`)로 가리킨다.**

```text
BTree {
  nodes: Vec<Node>     ← 노드 창고
  root:  NodeId        ← 지금 루트가 창고 몇 번인지
  max_keys: usize      ← 노드 하나의 최대 키 수 (이걸 넘으면 분할)
}

nodes:  [0]        [1]        [2]          [3]
        Leaf       Leaf       Internal     ...
        10,20      30,40      keys [30]
                              children [0, 1]
root = 2
```

- 새 노드는 항상 **맨 끝에 push** → 번호 = push 직전의 `nodes.len()`.
- 루트가 바뀌어도 노드는 움직이지 않는다. `root` 번호만 바뀐다.
- **2단계 연결고리**: 이 번호가 그대로 디스크 페이지 번호(`PageId`)가 된다. `nodes[id]` → `read_page(id)`.

---

## 3. 노드 두 종류

```text
Internal (이정표)                          Leaf (데이터)
┌───────────────────────────┐            ┌──────────────────────────┐
│ keys:     [ 30 | 50 ]     │            │ keys:   [ 30 | 40 ]      │
│ children: [ c0 | c1 | c2 ]│            │ values: [ v30| v40]      │
└───────────────────────────┘            │ prev ◀─  ─▶ next         │
  키 n개, 자식 n+1개                       └──────────────────────────┘
  값 없음 — 길 안내만                        값은 리프에만 있다
                                            옆 리프와 양방향 연결
```

**구분자 규칙: 왼쪽 < 구분자 ≤ 오른쪽**

```text
            [ 30 | 50 ]
           /     |     \
       key<30  30≤key<50  50≤key
```

키가 구분자와 같으면 **오른쪽**으로 간다. 그래서 리프 분할 때 "오른쪽 리프의 첫 키"를 구분자로 쓰면 규칙이 저절로 맞는다.

---

## 4. 찾기 (`get` · `find_leaf`) ✅

```text
get(40)
  root [30 | 50]   partition_point(sep <= 40) = 1  ("40 이하인 구분자 1개")
       → children[1]
  leaf [30, 40]    binary_search(40) = Ok(1)  → values[1]
```

- 내부 노드: `partition_point(|&sep| sep <= key)` = key 이하인 구분자 개수 = 내려갈 자식 칸.
- 리프: `binary_search` — `Ok(i)`면 값, `Err(_)`면 없음(`None`).
- 높이만큼만 내려간다. 높이는 `log_{fan-out}(N)` 정도로 낮게 유지된다.

---

## 5. 넣기 (`insert`) 전체 흐름 ✅

노드에 **부모 링크가 없다.** 대신 재귀 호출이 돌아오는 길이 부모에게 가는 길이다.

```text
insert(key)
 └ insert_at(root) ─────────────── Internal: 자식 칸 i 고르고 내려감
     └ insert_at(child) ────────── Leaf: 정렬 위치에 넣음
          넘침? 아니오 → (None, None) 반환
                예     → split_leaf → (None, Some((sep, 새 리프))) 반환
     ◀── Split을 받은 부모: keys.insert(i, sep), children.insert(i+1, 새 리프)
 ◀── 루트 자신이 Split을 돌려줬다 = 부모가 없다 → 새 루트를 만든다
```

반환 타입 `(Option<u64>, Option<Split>)`:
- 첫째 = 이전 값(같은 키가 있었으면 값을 바꾸고 옛 값을 돌려줌 = **upsert**, `mem::replace`).
- 둘째 = "나 갈라졌어" 소식 `Split = (구분자, 새 오른쪽 노드 번호)`.

---

## 6. 리프 분할 (`split_leaf`) — copy-up ✅

`max_keys = 3`, 리프 A에 40을 넣어 `[10,20,30,40]`(4개)로 넘침. A의 오른쪽 이웃은 C.

```text
① 반으로 자름  mid = 4/2 = 2,  keys.split_off(2)

   A [10, 20, 30, 40]   ──▶   A [10, 20]      A' [30, 40]   (A'는 nodes 맨 끝에 새로)

② 구분자 = A'의 첫 키 "복사"  sep = 30
   부모로 30이 올라가지만, 30은 A' 리프에도 그대로 남는다 (값이 리프에만 있으니까)

③ 링크 4개
   전:   A ⇄ C
   후:   A ⇄ A' ⇄ C

     A.next  = A'      next.replace(new_id) — 옛 next(C)는 old_next로 빼 둠
     A'.prev = A
     A'.next = C       old_next
     C.prev  = A'      C가 있을 때만 (맨 오른쪽 리프가 갈라지면 C 없음)

④ (30, A') 를 반환 → 부모가 받아서 끼움
```

**C.prev를 빼먹으면?** next 방향(오름차순)은 멀쩡하지만, prev 방향(내림차순 `range_rev`)이 C에서 A로 건너뛰어 **A'의 키 전체가 빠진다.** 테스트가 한쪽 방향만 보면 못 잡는 버그다.

---

## 7. 부모에 끼우기 ✅

```text
부모 keys [30 | 50]   children [L0, L1, L2]
L1(i = 1)이 갈라져서 (40, L1') 반환

keys.insert(1, 40)        → [30 | 40 | 50]
children.insert(2, L1')   → [L0, L1, L1', L2]
                                 ▲    ▲
                       갈라진 자식(i)  바로 오른쪽(i+1)에 새 자식
```

---

## 8. 루트 분할 — 깊이가 느는 유일한 곳 ✅

루트가 갈라지면 Split을 받을 부모가 없다. **새 내부 노드를 만들어 루트로 삼는다.**
(구분자 키 자체가 노드가 되는 게 아니라, 구분자 1개 + 자식 2개를 가진 **새 노드**가 생긴다.)

```text
전 (깊이 1, 루트 = 리프)              후 (깊이 2)

  [10, 20, 30, 40]  넘침                    [30]          ← 새 루트 Internal
                                          /      \
                                   [10,20]  ⇄  [30,40]

새 루트 = Internal { keys: [sep], children: [옛 루트, 새 노드] }
```

트리는 **위로** 자란다. 그래서 모든 리프는 항상 같은 깊이에 있다.

---

## 9. 성장 그림 — 2단계가 가득 차서 3단계로 (`max_keys = 3`, 10·20·30… 순서대로)

```text
10,20,30 넣음                    [10,20,30]                       깊이 1

40 → 리프 분할 + 루트 분할              [30]                         깊이 2
                                 [10,20] [30,40]

60 → 리프 분할                        [30 | 50]
                                 [10,20] [30,40] [50,60]

80 → 리프 분할                        [30 | 50 | 70]               루트 가득(3개)
                                 [10,20] [30,40] [50,60] [70,80]

100 → 리프 분할 → 루트에 90 추가        [30 | 50 | 70 | 90]  ← 4개, 넘침!
                                 [10,20] [30,40] [50,60] [70,80] [90,100]
```

**✅ diff 04 — 내부 노드 분할(push-up)** 로 여기서 루트가 갈라진다:

```text
넘친 내부 [30 | 50 | 70 | 90]   가운데 70을 위로 "이동" (복사 아님 — 내부엔 70이 남지 않음)

                         [70]                       ← 새 루트, 깊이 3
                       /      \
               [30 | 50]        [90]
              /    |    \       /   \
        [10,20] [30,40] [50,60] [70,80] [90,100]     ← 리프 링크는 그대로 ⇄
```

| | 리프 분할 (copy-up) | 내부 분할 (push-up) |
|---|---|---|
| 올라가는 키 | 오른쪽 리프의 첫 키 | 가운데 키 |
| 아래에 남나? | **남는다** (값이 리프에 있음) | **안 남는다** (이정표일 뿐) |
| 링크 | prev/next 4개 고침 | 없음 — 자식 목록만 나눔 |
| 상태 | ✅ diff 03 | ✅ diff 04 |

> diff 03까지는 내부 노드가 넘쳐도 그대로 뒀다. diff 04부터 `split_internal`이 같은 `Split`을 돌려줘 부모·루트 코드 변경 없이 위로 전파된다.

---

## 10. 범위 읽기 ✅ diff 05

```text
range(25, 65):   find_leaf(25) → 그 리프부터 next 따라 오른쪽으로, 65 넘으면 멈춤
                 [10,20] → [30,40] → [50,60] → [70,80]✗
                    25↑      30 40    50 60      70>65 stop
range_rev:       find_leaf(65) → prev 따라 왼쪽으로
```

내부 노드를 다시 타지 않고 **리프 연결만 따라간다** — B+tree가 범위 조회에 강한 이유.

---

## 11. 진행표

| diff | 내용 | 상태 |
|---|---|---|
| 01 | crate · `Index` trait · proptest dev-dep | ✅ |
| 02 | 노드 구조 · `get` · `find_leaf` | ✅ |
| 03 | `insert` · 리프 분할 · 링크 · 루트 분할 (+03b clippy·fmt) | ✅ |
| 04 | 내부 노드 분할 (push-up) | ✅ |
| 05 | `range` · `range_rev` · `impl Index for BTree` | ✅ |
| 06 | 테스트: 단위 + proptest(BTreeMap 비교) + 구조 검사 | ⏳ |
| 07~09 | Extendible Hash | ⏳ |
| 10 | 변이 테스트 · 마무리 | ⏳ |

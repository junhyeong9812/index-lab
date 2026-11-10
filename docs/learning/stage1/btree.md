# B+tree — 구조 · 알고리즘 그림 정리 (1단계, diff 06 기준)

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

## 10. 범위 읽기 (`range` · `range_rev`) ✅ diff 05

### 10-1. `range(lo, hi)` — lo의 리프에서 시작해 `next`를 따라감

```rust
let mut cur = Some(self.find_leaf(lo));            // 위에서 한 번만 내려옴
while let Some(id) = cur {
    let start = keys.partition_point(|&k| k < lo); // 이 리프에서 lo 이상이 시작되는 자리
    for (&k, &v) in keys[start..].iter().zip(&values[start..]) {
        if k > hi { return out; }                  // hi "보다 큰" 키에서 끝 (hi 자신은 포함)
        out.push((k, v));
    }
    cur = *next;                                   // 오른쪽 리프로 (None이면 체인 끝 → 종료)
}
```

```text
range(25, 65)
           [70]
        /        \
   [30|50]        [90]
  ↓find_leaf(25)
 [10,20] → [30,40] → [50,60] → [70,80] ✗
  25 미만    30 40     50 60     70 > 65 → 끝
결과 [30, 40, 50, 60]
```

- **시작**: `find_leaf(lo)` — lo가 있어야 할 리프.
- **이동**: `next` 링크로 오른쪽 리프.
- **정지**: 처음으로 `k > hi`인 키를 만날 때, 또는 `next == None`(체인 끝). 양끝 포함 `[lo, hi]`(T14).
- `start`는 첫 리프에서만 의미 있다 — 다음 리프부터는 키가 전부 ≥ lo라 `start = 0`.
- `lo > hi`면 빈 결과(01a의 약속).

### 10-2. `range_rev(lo, hi)` — 거울처럼 반대로

```rust
let mut cur = Some(self.find_leaf(hi));            // hi의 리프에서 시작
let end = keys.partition_point(|&k| k <= hi);      // 이 리프에서 hi 이하가 끝나는 자리
for (...) in keys[..end]...rev() {                 // 뒤에서부터 → 내림차순
    if k < lo { return out; }                      // lo "보다 작은" 키에서 끝
    out.push((k, v));
}
cur = *prev;                                       // 왼쪽 리프로
```

| | `range` | `range_rev` |
|---|---|---|
| 시작 리프 | `find_leaf(lo)` | `find_leaf(hi)` |
| 따라가는 링크 | `next` | `prev` |
| 출력 순서 | 오름차순 | 내림차순(`.rev()`) |
| 정지 | 첫 `k > hi` | 첫 `k < lo` |

diff 03에서 맞춘 **`prev` 링크와 `C.prev = A'`가 처음 실제로 쓰이는 곳.** C.prev를 안 고쳤다면 C의 prev가 여전히 A를 가리켜 `range_rev`가 A'를 건너뛰고 A'의 키가 통째로 빠진다(정순 `range`는 영향 없음).

### 10-3. 내부 노드는 몇 번 거치나

- 루트 → 리프로 **한 번** 내려간다(`find_leaf` 1회). 그 길에 높이만큼 내부 노드를 지난다 — 위 그림에선 `[70]`, `[30|50]` 2개.
- 그 뒤로는 **리프 → 리프로 바로** 옮겨 다니고 내부 노드로 다시 올라가지 않는다.
- 그래서 범위 조회 비용 ≈ 높이(내려가기 1회) + 읽은 리프 수. B+tree가 범위 조회에 강한 이유.

### 10-4. 역순은 왜 필요한가 — 결과는 같고, "끝까지 읽느냐"가 다르다

`range_rev(lo, hi)` = `range(lo, hi)`를 뒤집은 것과 **같은 결과**다. 차이는 읽는 양:

```sql
SELECT * FROM orders WHERE id <= 1000000 ORDER BY id DESC LIMIT 10;   -- 최신 10개
```

| 방법 | 읽는 양 |
|---|---|
| 정순으로 전부 → 뒤집기 → 10개 | 100만 개 전부 |
| 역순: `find_leaf(1000000)`에서 `prev`로 10개 읽고 멈춤 | 리프 한두 개 |

- 근거(명세): T7 양방향 리프(InnoDB `FIL_PAGE_PREV/NEXT`·PostgreSQL `btpo_prev/next` — 역방향 스캔에 사용), T13 `range_rev`를 trait에(Hash와의 실험 축).
- ⚠️ 지금은 `Vec`으로 전부 모아 돌려주므로 "N개만 읽고 멈추기"는 아직 못 한다 — iterator화는 다음 작업 후보.

### 10-5. `C.prev`는 왜 꼭 고쳐야 하나

| 방향 | 책임 링크 | 안 고치면 |
|---|---|---|
| 정순 `next` | `A.next = A'`, `A'.next = C` | `range`가 A'를 건너뜀 |
| 역순 `prev` | `C.prev = A'`, `A'.prev = A` | `range_rev`가 A'를 건너뜀 |

각 방향 체인에서 A'로 들어가는 길은 **하나뿐**(next 쪽 A.next, prev 쪽 C.prev). 불변식 I2: next로도 prev로도 모든 키를 한 번씩, `a.next = b` ⇔ `b.prev = a`.
단방향이었다면 C.prev도 없다 — 양방향의 대가 = 분할 때 C를 한 번 더 쓰기(2단계에서 페이지 쓰기 +1로 측정).

### 10-6. `impl Index for BTree`

```rust
impl super::Index for BTree {
    fn insert(&mut self, key: u64, value: u64) -> Option<u64> { BTree::insert(self, key, value) }
    fn get(...)       { BTree::get(self, key) }
    fn range(...)     { BTree::range(self, lo, hi) }
    fn range_rev(...) { BTree::range_rev(self, lo, hi) }
}
```

- 01a의 약속(trait)을 B+tree가 지킨다는 선언 → `Box<dyn Index>`로 Hash와 바꿔 끼울 수 있다(3단계 러너).
- 몸통은 이미 있는 메서드 호출뿐. `BTree::insert(self, …)`처럼 경로를 다 쓰는 건 trait 메서드가 자기 자신을 재귀 호출하는 실수를 피하려는 것.

---

## 11. 테스트와 구조 검사 ✅ diff 06

### 11-1. 어떻게 만들었나 — 약속에서 출발한 테스트

- 테스트 설계는 **구현 코드를 보지 않은 별도 워커**가 명세(I1·I2·I4)·trait·구조체 정의만 보고 했다.
  "코드가 이렇게 생겼으니 이렇게 테스트"가 아니라 "**약속이 이러니 이걸 확인**"에서 출발.
- 명세 T12대로 `depth()`·`check()`는 **공개 API**(`pub fn`) — 테스트와 3단계 실험이 같이 쓴다.

### 11-2. `pub fn check(&self)` — 트리가 B+tree 모양인지 전수 검사 (I2)

트리를 한 번 돌며 하나라도 어긋나면 이유와 함께 panic.

```text
노드마다 (walk)                           리프 사슬 전체
 ├ 한 번만 방문 (공유·순환·고아 노드 없음)    ├ 모든 리프 깊이 같음
 ├ 키 수 ≤ max_keys, 엄격 오름차순          ├ 첫 리프 prev = None, 끝 리프 next = None
 ├ 구분자 범위 [lo, hi) 안에 키가 있음      ├ a.next = b ⇔ b.prev = a
 ├ 비루트 하한: 리프 ⌈m/2⌉, 내부 ⌊m/2⌋     ├ next 사슬 = 트리를 왼→오로 본 리프 순서
 ├ 내부: 자식 = 키 + 1                     └ prev 사슬 = 그 역순
 └ 구분자 = 오른쪽 서브트리의 최소 키
```

**구분자 범위는 위에서 물려받는다.** 부모를 지날 때마다 칸막이가 허용 범위 `[lo, hi)`를 좁힌다.

```text
           [70]                  왼쪽 자식 [−, 70) · 오른쪽 [70, −)
        /        \
   [30|50]        [90]           [30|50] 아래: [−, 30) · [30, 50) · [50, 70)
```

```rust
let clo = if i == 0 { lo } else { Some(keys[i - 1]) };   // i번 자식의 아래 경계 = 왼쪽 칸막이(없으면 물려받은 lo)
let chi = if i < keys.len() { Some(keys[i]) } else { hi }; // 위 경계 = 오른쪽 칸막이(없으면 물려받은 hi)
let min = walk(t, c, depth + 1, clo, chi, w);
```

범위를 벗어난 키가 하나라도 있으면 "왼쪽 < 구분자 ≤ 오른쪽"이 깨진 것. 덤으로 "구분자 = 오른쪽 서브트리의 최소 키"도 확인한다(copy-up·push-up이고 삭제가 없으니 성립 — 삭제가 생기면 완화해야 함).

**하한 ⌈m/2⌉ · ⌊m/2⌋의 출처**

- 리프: max+1개를 반으로 → 작은 쪽도 ⌈m/2⌉개.
- 내부: max+1개 중 가운데 1개를 올리고 남은 max개를 반으로 → ⌊m/2⌋개.
- 삭제가 없으니 이보다 작아질 일이 없다. (명세의 "⌈max/2⌉ − 1 수준"보다 촘촘한 값.)

`pub fn depth()` — 맨 왼쪽 길로 내려가며 센 노드 수. 3단계에서 "fan-out에 따라 높이가 어떻게 되나"를 잴 때 쓴다.

### 11-3. 단위 테스트 13개 — 손으로 그린 장면을 그대로 확인

| 테스트 | 확인하는 것 |
|---|---|
| `first_leaf_split_copies_up_separator` | 10~40 → 루트 `[30]`, 리프 `[10,20]`·`[30,40]`, 30이 리프에 남음(copy-up), 링크 4방향 |
| `internal_split_pushes_up_middle_and_grows_root` | 10~100 → 루트 `[70]`, `[30,50]`·`[90]`, 깊이 3 (9절 그림) |
| `upsert_*` 3개 | 이전 값 반환, 꽉 찬 리프에 upsert해도 **노드가 새로 안 생김** |
| `range_boundaries` · `keys_at_u64_extremes` | 양끝 포함, `lo > hi`, `lo == hi`, 키 밖 범위, `u64::MAX` 키 |
| `sequential_and_reverse_insertion` | m = 2~6, 500개 정순·역순 → 높이 ≥ 3, 결과 일치 |
| 그 밖 | 빈 트리, `new(1)` panic, m = 2, `dyn Index` 경유 |

### 11-4. proptest 2개 — 무작위로 수백 번 넣고 정답지와 대조 (I1·I4)

```rust
for &(k, v) in &ops {
    prop_assert_eq!(t.insert(k, v), map.insert(k, v));   // 반환값까지 BTreeMap과 같아야
}
t.check();
// get · range · range_rev를 무작위 구간으로 BTreeMap과 대조 (dyn Index로도 한 번 더)
```

- **정답지** = `std::collections::BTreeMap` — 이미 검증된 표준 구현과 결과를 비교한다.
- `matches_btreemap`: `max_keys` 2~8 · 연산 ≤ 400 · 256회.
- `structure_holds_after_every_insert`: **삽입할 때마다** `check()` — 분할 직후의 중간 상태도 규칙을 지키는지. (끝에서 한 번만 보면, 중간에 깨졌다가 우연히 맞춰진 상태나 어느 삽입에서 깨졌는지를 놓친다.)
- 키 전략: 좁은 범위 0~200을 6할로 → 중복·upsert가 자주 일어나게 · 나머지는 `u64::MAX` 근처와 전체 범위.
- 실패하면 proptest가 실패 입력을 **가장 짧게 줄여서(shrink)** 보여준다.

### 11-5. 테스트가 정말 잡는가

- `cargo test` 15/15 (≈1.1초) · `clippy -D warnings` · `fmt --check` 통과.
- **맛보기 변이**: `find_leaf`의 `sep <= key`를 `sep < key`로 틀리게 → 테스트 4개 실패(검출).
- `check()` 자체도 일부러 망가뜨린 트리 4종(prev 끊기·구분자 −1·고아 노드·키가 모자란 리프)으로 모두 잡는 것을 확인.
- 정식 변이 점검 2건은 diff 10.

---

## 12. 진행표

| diff | 내용 | 상태 |
|---|---|---|
| 01 | crate · `Index` trait · proptest dev-dep | ✅ |
| 02 | 노드 구조 · `get` · `find_leaf` | ✅ |
| 03 | `insert` · 리프 분할 · 링크 · 루트 분할 (+03b clippy·fmt) | ✅ |
| 04 | 내부 노드 분할 (push-up) | ✅ |
| 05 | `range` · `range_rev` · `impl Index for BTree` | ✅ |
| 06 | 테스트: 단위 + proptest(BTreeMap 비교) + 구조 검사 | ✅ (이해 게이트 대기) |
| 07~09 | Extendible Hash | ⏳ |
| 10 | 변이 테스트 · 마무리 | ⏳ |

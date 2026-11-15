# 2단계 — 페이지 B+tree: 블라인드 테스트와 구조 검사 check() (diff 08)

## 1. 범위

- `tests/paged_btree.rs` (964줄 · 31 테스트) — 구현(`paged_btree.rs`)을 보지 않은 워커가 명세와 공개 API만 보고 작성.
  - 예제 22개: create/open 경계, 리프·내부 분할, 2프레임 풀, 핀 누수 0, 재열기.
  - proptest 2개: Mem 256케이스(max_keys 2..=6 · 프레임 2..=4 · 매 삽입 후 `check`) · File 24케이스 + 재열기.
  - `check()` 보호 테스트 7개: 일부러 망가뜨린 트리를 `check()`가 거부하는지 확인.
- `check()` = 트리 전체를 걸으며 구조 불변식(I2)과 메타 일치를 검사. 핀은 한 번에 1장.

## 2. 구분자 범위 `[lo, hi)` — 게이트 Q1 (2/2 fail → 설명 후 통과)

`tests/paged_btree.rs:89` · `:162-166`

```rust
let in_bounds = |k: u64| lo.is_none_or(|l| l <= k) && hi.is_none_or(|h| k < h);
...
let clo = if i == 0 { lo } else { Some(keys[i - 1]) };
let chi = if i == keys.len() { hi } else { Some(keys[i]) };
walk(pool, c, depth + 1, height, max_keys, false, clo, chi, w);
```

`get`의 라우팅 (`src/index/paged_btree.rs:61`):

```rust
id = node.children()[node.keys().partition_point(|&sep| sep <= key)];
```

| 함수 | `keys = [30]`, key = 30 | 뜻 |
|---|---|---|
| `binary_search(&30)` | `Ok(0)` | 30이 **0번 칸에 있다** (위치) |
| `partition_point(\|&sep\| sep <= 30)` | **1** | `sep <= 30`인 구분자가 **1개** (개수) → `children()[1]` = c1 |

인과:
1. 리프 분할 구분자 = **오른쪽 리프의 첫 키**(`right.keys[0]`) → 30은 오른쪽 리프에 있다.
2. 그래서 `get`도 "구분자와 같은 키는 오른쪽"(`sep <= key`).
3. 자식 c_i의 범위: 아래 경계(왼쪽 구분자)는 그 자식의 첫 키일 수 있어 **포함**(`l <= k`), 위 경계(오른쪽 구분자)는 오른쪽 형제 몫이라 **제외**(`k < h`).
4. 위를 `<=`로 느슨하게 하면 c0에 있는 30도 통과 → 그런데 `get(30)`은 c1로만 가므로 **영영 못 찾는 키**. 이걸 잡으려고 `<`.

```
            [30]
           /    \
     c0 [10, 30)   c1 [30, 90)
     10 20         30 40 50      ← 30은 c1에만 있어야 get이 찾는다
```

### 오답 노트

| 내 답 | 실제 |
|---|---|
| "`=`를 빼서 경계값일 때 오른쪽을 한 번 더 보는 걸 줄이는 효율 개선" | 효율이 아니라 **정확성**. 위 경계를 포함하면 `get`이 못 찾는 키를 `check`가 통과시킨다 |
| "`partition_point`가 0 → c0로 내려간다" | `partition_point`는 위치가 아니라 **조건을 만족하는 개수**. `30 <= 30` 참 → 1 → c1. 위치를 주는 건 `binary_search` |

## 3. check()를 검사하는 테스트 — 게이트 Q2 (pass, 사전 점검 1회)

`tests/paged_btree.rs:897-906` · `:958-964`

```rust
fn check_rejects(corrupt: impl Fn(&BufferPool<MemStorage>, &Shape, PageId)) {
    let mut t = mem_tree(3, 4);
    for k in 0..40u64 { t.insert(k * 2, k).unwrap(); }
    let shape = check(&t, 3);                 // ① 정상 트리 → 통과
    corrupt(t.pool(), &shape, t.root());      // ② 일부러 망가뜨림
    let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| check(&t, 3)));
    assert!(r.is_err(), "check() accepted a corrupted tree");   // ③ 거부해야 통과
}

fn check_detects_shared_child() {
    check_rejects(|pool, _, root| {
        let mut g = pool.fetch_mut(root).unwrap();
        let n: &mut InternalPage = bytemuck::cast_mut(&mut *g);
        n.children[1] = n.children[0];
    });
}
```

- **왜 따로 검사하나**: 다른 테스트는 전부 "`check()` 통과 = 트리 정상"을 믿는다. `check()`가 무엇이든 통과시키면(fail-open) 31개가 망가진 트리 위에서도 **전부 초록** — 테스트가 의미를 잃는다. 보호 테스트 7개(prev/next 링크 · 정렬 · 구분자 범위 · 과적 · 메타 높이 · 공유 자식)가 `check()`의 각 검사가 실제로 작동함을 보증한다.
- **공유 자식 손상 ↔ diff 07 변이 `left.count = mid + 1`** (n = 3, `mid = n / 2 = 1` — 정수 나눗셈, 변이는 `left.count` 한 줄만 바꾼다):

  | | 왼쪽 `children()` | 오른쪽 `children()` |
  |---|---|---|
  | 정상 (count 1) | c0 c1 | c2 c3 |
  | 변이 (count 2) | c0 c1 **c2** | **c2** c3 |

  공통점: **한 페이지에 두 경로로 도달한다.** 차이: 테스트 손상은 한 노드 안의 두 슬롯이, 변이는 서로 다른 두 부모가 같은 자식을 가리킨다. `walk`의 `seen` HashSet(`w.seen.insert(id)` 실패 → "reachable twice")이 둘 다 잡는다.

## 4. 변이 × 블라인드 테스트

| 변이 | 실패한 테스트 수 |
|---|---|
| 리프 분할 `C.prev` 수정 생략 | 8 |
| 구분자 = 왼쪽 최대 키 | 21 |
| 내부 분할 copy-up(`left.count = mid + 1`) | 18 |
| `range` 시작 = 0 | 7 |
| `range_rev` 끝 = len | 4 |
| `open` magic 검사 생략 | 1 |
| 메타 height 미갱신 | 20 |
| 리프 분할 off-by-one(max+1까지 허용) | 18 |

8종 전부 검출. 전체 64 + 33 + 31 green · clippy · fmt 통과.

## 5. 진행

| diff | 게이트 |
|---|---|
| 08 | Q1 ❌❌ → 설명 후 통과 (partition_point = 개수 · `[lo, hi)`) · Q2 ✅ (사전 점검 1회) |

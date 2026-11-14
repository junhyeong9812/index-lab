# 2단계 — 페이지 B+tree: 조회·삽입·리프 분할 (diff 06)

## 1. 범위

- `src/index/paged_btree.rs` `PagedBTree`: `create` · `get` · `insert`(리프 분할 + **리프 루트의 루트 분할**까지).
- 내부 노드 분할(push-up)은 diff 07 — 지금은 `unimplemented!`.
- `btree_page.rs`에 칸을 밀어 끼우는 `LeafPage::insert_at` · `InternalPage::insert_at`(`copy_within`) 추가.
- `create`는 `2 ≤ max_keys < 1022`를 요구한다. "넣고 넘치면 나눈다" 방식이라 노드가 잠깐 `max_keys + 1`개를 들고 있어야 하고, 그 한 칸이 페이지에 들어가야 한다.

## 2. 부모 핀을 놓고 내려간다 (S5) — 게이트 Q1

`src/index/paged_btree.rs:117-139`

```rust
let (i, child) = {
    let page = self.pool.fetch(id)?;
    let node: &InternalPage = bytemuck::cast_ref(&*page);
    let i = node.keys().partition_point(|&sep| sep <= key);
    (i, node.children()[i])
}; // 부모 핀을 여기서 놓고 내려간다(S5)                       ← :126
let (old, split) = self.insert_at(child, key, value)?;   // 재귀
let Some((sep, right)) = split else { return Ok((old, None)); };
let mut page = self.pool.fetch_mut(id)?;                  // ← :132 부모를 다시 fetch
let node: &mut InternalPage = bytemuck::cast_mut(&mut *page);
node.insert_at(i, sep, right);
```

| 방식 | 동시 핀 | 비용 |
|---|---|---|
| 부모 핀을 쥔 채 재귀 | 경로 전체 h+1개(분할 시 `new_page`로 +1) → 작은 풀에서 `AllPinned` | 다시 읽을 필요 없음 |
| **놓고 재귀 → 분할 시 재-fetch** | 최대 2개 | 부모가 핀 0 → `take_frame`의 내보내기 후보. 남아 있으면 hit(read 0), 내보내졌으면 miss → 디스크 read 1회(그 프레임이 dirty였다면 내보낼 때 write도 1회) |

- `take_frame`은 `pins == 0`인 프레임 중 `last_used`가 가장 작은 것을 고르고, 하나도 없으면 `AllPinned`를 돌려준다.
- 변이 "부모 핀을 쥔 채 재귀" → **컴파일 오류 E0502**. `insert_at`이 `&mut self`라서 `self.pool`을 빌린 가드를 쥔 채 재귀할 수 없다. S5를 빌림 검사기가 강제한다.

## 3. 리프 분할과 형제 링크 — 게이트 Q2

`src/index/paged_btree.rs:166-189` (요지)

```rust
let (new_id, sep, old_next) = {
    let mut left_page = self.pool.fetch_mut(id)?;          // A
    let (new_id, mut right_page) = self.pool.new_page()?; // A' — 핀 2개
    // keys/values[mid..n] → A', left.count = mid
    right.prev = id; right.next = old_next; left.next = new_id;
    (new_id, right.keys[0], old_next)                      // 구분자 = A'의 첫 키(copy-up)
}; // A·A' 핀을 놓은 뒤 C를 잡는다                          ← :183
if old_next != NO_PAGE {
    let mut c = self.pool.fetch_mut(old_next)?;            // C
    c.prev = new_id;                                       // C.prev = A'
}
```

`A ⇄ C` → `A ⇄ A' ⇄ C`. 링크 4개: `A'.prev = A`, `A'.next = C`, `A.next = A'`, `C.prev = A'`.

- **`C.prev` 수정을 빼먹으면**: C.prev가 여전히 A → 역순회(prev 따라가기)가 C에서 A로 바로 넘어가 **A'의 키가 결과에서 빠진다**. 정순회는 멀쩡해서 앞방향 테스트만으로는 못 잡는다(변이: 1개 실패 — `middle_split_fixes_right_neighbours_prev`).
- **C를 블록 밖에서 잡는 이유**: A·A'를 쥔 채 C를 fetch하면 핀 3개. 프레임 2개 풀에서는 `take_frame`이 `pins == 0`인 victim을 못 찾아 `AllPinned`. 블록을 끝내 A·A'를 놓고 잡으면 동시 핀 ≤ 2.
- 구분자 변이(= 왼쪽 최대 키) → 2개 실패: `get`이 `partition_point(|&sep| sep <= key)`로 "구분자 이상은 오른쪽"을 가정하므로 구분자는 오른쪽의 첫 키여야 한다.

## 4. 진행

| diff | 게이트 |
|---|---|
| 06 | ✅ Q1 pass(사전 점검 힌트 1회 — 재-fetch의 hit/miss) · Q2 pass |

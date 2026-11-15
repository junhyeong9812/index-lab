# 2단계 — 페이지 B+tree: 내부 분할·range·open·impl Index (diff 07)

## 1. 범위

- `split_internal`(push-up)이 diff 06의 `unimplemented!` 자리를 채운다. 분할은 위층 `insert_at`으로 연쇄되고, 루트까지 넘치면 diff 06의 루트 성장이 받는다.
- `range`·`range_rev`: 시작 리프를 찾은 뒤 `next`/`prev`만 따라간다. 핀은 한 번에 1장이다.
- `open`: 메타 페이지에서 root·height·max_keys를 복원한다. magic이 다르거나 max_keys가 범위를 벗어나면 `InvalidData`.
- `impl Index`: 오류는 `expect`로 panic(명세 Q6).
- A2 스모크: 프레임 4, max_keys 2로 높이 5까지 키운다. 결과는 BTreeMap과 같고 핀 누수 0, 내보내기 발생. 실제 동시 핀 상한은 2.

## 2. 리프 copy-up vs 내부 push-up — 게이트 Q1 (2/2 fail → 설명 후 통과)

`src/index/paged_btree.rs` 리프 분할(`:172-182` 요지)과 내부 분할(`:203-207`):

```rust
// 리프: keys[mid..n]과 values가 오른쪽 리프로, 구분자는 오른쪽 첫 키의 "사본"
right.keys[..n - mid].copy_from_slice(&left.keys[mid..n]);
(new_id, right.keys[0], old_next)

// 내부: 가운데 키를 부모로 "이동" — 왼쪽에서 빠진다
let sep = left.keys[mid];
right.keys[..n - mid - 1].copy_from_slice(&left.keys[mid + 1..n]);
right.children[..n - mid].copy_from_slice(&left.children[mid + 1..=n]);
left.count = mid as u64;
```

| | 리프 분할 (copy-up) | 내부 분할 (push-up) |
|---|---|---|
| 올라가는 것 | 오른쪽 리프 첫 키의 **사본** | 내부 노드 가운데 키(이미 사본) |
| 아래에 남나 | **남는다**. 원본과 값이 리프에 있다 | **안 남는다** (`left.count = mid`) |
| 이유 | 값은 리프에만 있다(`LeafPage.values`). 키를 빼면 `get`의 리프 `binary_search`가 실패해 `None`이 되고, `next`를 따라가는 `range`에서도 사라진다 | 내부 키는 값이 없는 길 안내다. 한 단계 위에서 같은 안내를 하니 잃는 게 없다 |

한 문장: **원본(값)은 리프에 딱 한 번, 내부 키는 길 안내용 사본이라 옮겨도 된다.**

### 변이 `left.count = mid + 1`에서 생기는 일 (n = 3, mid = 1)

분할해도 왼쪽 **배열 내용은 지우지 않는다**. `count`만 바뀌고, `keys()`·`children()`은 **칸 0부터** `count`개·`count+1`개를 잘라 보여 준다.

```
칸:            [0] [1] [2] [3]
left.keys    =  k0  k1  k2  —
left.children=  c0  c1  c2  c3
```

| | 정상 (count 1) | 변이 (count 2) |
|---|---|---|
| 왼쪽 keys / children | k0 / c0 c1 | k0 k1 / **c0 c1 c2** |
| 부모로 | k1 | k1 |
| 오른쪽 keys / children | k2 / c2 c3 | k2 / c2 c3 |

- 노드 하나만 보면 `자식 = 키 + 1`은 변이에서도 지켜진다. 깨지는 건 **노드 사이**다.
- 자식 c2(예: 20~29 구역)를 왼쪽과 오른쪽이 **둘 다** 가리킨다. 부모는 "20 이상은 오른쪽"이라 안내하므로 왼쪽의 c2 포인터는 거짓이고, 나중에 c2가 분할되면 낡은 채 남는다.
- k1이 부모와 왼쪽에 **중복**된다. 테스트 `internal_split_pushes_middle_key_up`이 "내부 키 중복 없음"으로 잡는다.

### 오답 노트

| 내 답 | 실제 |
|---|---|
| "구분자 키가 있어야 리프인 걸 안다" | 리프/내부 구분은 첫 칸 `kind`(`kind_of`). 구분자를 남기는 이유는 **값이 리프에만 있어서** |
| "변이에서 왼쪽 자식은 c0, c1, c3" | `children()`은 칸 0부터 **연속**. 중간을 건너뛰지 않는다 → c0 c1 c2 |
| "count=2면 k0 왼쪽에 값이 들어가나 / 끝에서 앞으로 도나?" | 아무것도 새로 쓰지 않는다. 배열은 그대로고 count는 "앞에서부터 몇 칸이 살아 있나"일 뿐 |
| "카운트당 키 1, 자식 2 고정" | 키 count개 · 자식 count+1개 (count=2면 키 2, 자식 3) |
| "30의 값은 왼쪽 리프에 남는다?" | `keys[mid..n]`이 오른쪽으로 → 30은 **오른쪽** 리프. 루트의 30은 값 없는 사본 |
| "내부에서 옮긴다고? 리프에 남기는 게 맞지 않나?" | 둘 다 맞다. 리프의 원본은 그대로, 내부 노드에 있던 **사본**만 위로 이동 |

## 3. `open`의 magic 검사와 살아남은 변이 — 게이트 Q2 (pass)

`src/index/paged_btree.rs:282-297`

```rust
if magic != BTREE_MAGIC {                                          // 검사 ①
    return Err(invalid("page 0 is not a B+tree meta page".into()));
}
if !(2..LEAF_CAPACITY.min(INTERNAL_CAPACITY)).contains(&max_keys) { // 검사 ② (2 ≤ max_keys < 1022)
    return Err(invalid(format!("meta max_keys {max_keys} out of range")));
}
```

- 처음 테스트는 0으로 찬 페이지 0을 썼다. 그 페이지는 `max_keys = 0`이라 검사 ②가 거부한다 → 검사 ①을 지워도 테스트가 통과(변이 생존). 테스트가 **확인하려던 검사를 실제로 거치지 않았다**.
- 보강: 다른 값은 멀쩡하고 magic만 `IDXHASH_`인 메타 → 검사 ①만이 잡는다.
- magic이 없으면: 페이지 0의 칸 3에 우연히 2~1021이 있는 다른 파일(예: Hash 인덱스 파일)을 `open`이 받아들이고, 이후 `get`·`range`가 Hash 페이지를 B+tree 노드로 해석해 엉뚱한 값을 읽거나 손상 오류를 낸다.

## 4. 진행

| diff | 게이트 |
|---|---|
| 07 | Q1 ❌❌ → 설명 후 통과 (copy-up vs push-up · count 슬라이스) · Q2 ✅ (사전 점검 1회) |

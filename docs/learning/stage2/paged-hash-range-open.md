# 2단계 — 페이지 Extendible Hash: range·open·check·Index + 블라인드 테스트 (diff 11)

## 1. 범위

`src/index/paged_hash.rs`

| 함수 | 하는 일 |
|---|---|
| `range` · `range_rev` | 서로 다른 버킷을 전부 훑어 범위로 거른 뒤 정렬, `range_rev`는 뒤집기(1단계 T10·T13) |
| `buckets()` | 디렉터리 페이지를 한 장씩 읽어 **쓰는 칸만** 복사 → `sort` + `dedup` (핀 1) |
| `open` | magic · `bucket_cap` 범위 · 전역 깊이 ≤ 21 · 디렉터리 페이지 수 = ceil(2^전역 / 2048) 검사 → `InvalidData` |
| `check()` | I3 구조 불변식 + 메타 일치(공개 — 테스트·3단계 실험 공용) |
| `bucket_count` · `impl Index` | 서로 다른 버킷 수 · 오류는 `expect`로 panic(명세 Q6) |

`tests/paged_hash.rs` (블라인드 워커, 구현 미열람): 40 테스트 — 예제 21 · proptest 2(Mem 256케이스·File 24케이스 + 재열기) · open 거부 7 · check() 손상 가드 9 + 하네스 가드 1.

## 2. `buckets()`의 `n`과 `dedup` — 게이트 Q1 (1/2 fail → 재시도 pass)

`src/index/paged_hash.rs:232-244`

```rust
fn buckets(&self) -> Result<Vec<PageId>> {
    let len = 1usize << self.global_depth;
    let mut ids = Vec::with_capacity(len);
    for (p, &dir_id) in self.dir_pages.iter().enumerate() {
        let page = self.pool.fetch(dir_id)?;
        let dir: &DirectoryPage = bytemuck::cast_ref(&*page);
        let n = (len - p * DIR_SLOTS).min(DIR_SLOTS);   // 이 페이지에서 쓰는 칸 수
        ids.extend_from_slice(&dir.slots[..n]);
    }
    ids.sort_unstable();
    ids.dedup();
    Ok(ids)
}
```

- **`n`**: 전역 깊이 2면 `n = 4`. 2048칸을 다 읽으면 쓰지 않는 칸은 0 → **0번 = 메타 페이지**를 버킷으로 읽는다.
- **`dedup`**: 칸 A, A, B, B → dedup 없으면 `range(0, 100)` =
  `[(10,100), (10,100), (20,200), (20,200)]`, `BTreeMap`은 `[(10,100), (20,200)]`.
  **느려지는 게 아니라 결과가 틀린다.** 219줄 `sort_unstable_by_key`는 순서만 맞출 뿐 중복을 지우지 않는다 — 주석 "키 중복이 없으니 불안정 정렬로 충분"은 `dedup`이 만든 전제다.

### 오답 노트

| 내 답 | 실제 |
|---|---|
| "중복이면 4번 탐색해야 된다" | 비용보다 **정확성**: 같은 버킷을 두 번 훑으면 키가 두 번씩 결과에 들어가 정답지와 달라진다 |
| "10,100,10,100 이런식" | 루프는 `[A, A, B, B]` 끝까지 돈다 — B의 `(20,200)`도 두 번 |

## 3. `open`의 디렉터리 페이지 수 검사 — 게이트 Q2 (pass)

`src/index/paged_hash.rs:272-277` · `bucket_of`

```rust
if dir_pages.len() != (1usize << global_depth).div_ceil(DIR_SLOTS) {
    return Err(invalid(...));
}
...
let page = self.pool.fetch(self.dir_pages[slot / DIR_SLOTS])?;   // bucket_of
```

메타가 전역 깊이 12(칸 4096)인데 `dir_count` 1이면:

- 검사가 없으면 `open` 성공 → slot 3000인 키에서 `dir_pages[1]` **범위 밖 panic**. `Result`를 돌려주는 `get`이지만 `Err`가 아니라 프로세스가 죽는다.
- slot < 2048인 키는 정상 동작 → 손상이 **잠복**했다가 일부 키에서만, 나중에 터진다.
- `open`에서 막으면 **연 직후, 항상, 호출자가 처리할 수 있는 `Err(InvalidData)`**로 드러난다.
- `open`의 `.min(MAX_DIR_PAGES)`(253줄)는 메타 슬라이스 panic만 막는 클램프 — 일관성은 ④ 검사가 따로 맡는다.

## 4. 변이

| 변이 | 단위 테스트 12개 | + 블라인드 테스트 40개 |
|---|---|---|
| `range` 상한 `<` | **생존** | 5 실패 |
| `range_rev` 뒤집기 생략 | **생존** | 10 실패 |
| `dedup` 생략 | 3 실패 | 24 실패 |
| 쓰지 않는 칸까지 읽기 | 2 실패 | — |
| `open` magic 검사 생략 | — | 1 실패 |
| `open` 디렉터리 페이지 수 검사 생략 | — | 1 실패 |
| 재지정 시작 = 접미사 (diff 10) | 4 실패 | 24 실패 |
| 메타 미기록 (diff 10) | 4 실패 | 24 실패 |
| 디렉터리 페이지 항상 0 (diff 09) | 1 실패 | 7 실패 |

전체 78 + 33 + 31 + 40 green · clippy · fmt 통과.

## 5. 진행

| diff | 게이트 |
|---|---|
| 11 | Q1 ❌ → ✅ (재시도 — 결과 중복 = 정확성) · Q2 ✅ (사전 점검 1회) |

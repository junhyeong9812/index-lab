# 2단계 — 페이지 Extendible Hash: 레이아웃과 get (diff 09)

## 1. 범위

- `src/index/hash_page.rs`: 페이지 3종(S3). 모두 16KB, `u64` 2048칸.

  | 페이지 | 헤더 | 본문 |
  |---|---|---|
  | 메타 (0번) | magic `IDXHASH_` · global_depth · bucket_cap · dir_count | 디렉터리 페이지 번호 최대 2044개 |
  | 디렉터리 | 없음 | 칸 2048개, 칸마다 버킷 페이지 번호 |
  | 버킷 | local_depth · count | 키·값 최대 1023쌍 (순서 없음) |

- `src/index/paged_hash.rs`: `create`(메타 0 · 디렉터리 1 · 빈 버킷 2, 핀 1개씩 순차) · `get`.
- `hash::mix`를 `pub(crate)`로 — 1단계와 같은 해시를 공유(동작 무변경).
- 디렉터리 최대 2044 × 2048칸 ≥ 2^21 → 전역 깊이 21까지(diff 10의 두 배 상한, 컴파일 단언).

## 2. 조회 경로 — 두 단계 간접 참조 + 핀 1개 (게이트 Q1 pass · Q2 pass)

`src/index/paged_hash.rs:58-80`

```rust
pub fn get(&self, key: u64) -> Result<Option<u64>> {
    let bucket_id = self.bucket_of(key)?;          // 디렉터리 핀은 여기서 이미 풀렸다
    let page = self.pool.fetch(bucket_id)?;
    ...
}

fn bucket_of(&self, key: u64) -> Result<PageId> {
    let slot = self.slot(key);                                        // mix(key)의 하위 global_depth 비트
    let page = self.pool.fetch(self.dir_pages[slot / DIR_SLOTS])?;    // ① 몇 번째 디렉터리 페이지
    let dir: &DirectoryPage = bytemuck::cast_ref(&*page);
    Ok(dir.slots[slot % DIR_SLOTS])                                   // ② 그 페이지의 몇 번째 칸
}
```

| 예 | `slot / 2048` (자리) | `dir_pages[자리]` (페이지 번호) | `slot % 2048` (칸) |
|---|---|---|---|
| slot 3000, `dir_pages = [1, 5]` | 1 | **5** | 952 |
| slot 5000, `dir_pages = [1, 5, 9]` | 2 | **9** | 904 |

- **왜 `1 + slot / 2048`로 계산하지 않나**: `new_page()`는 번호를 차례로 준다. 전역 깊이가 11 → 12로 커져 두 번째 디렉터리 페이지가 필요할 때쯤이면, 그 사이 분할로 생긴 **버킷 페이지들이 번호를 차지**했다. 디렉터리 페이지 번호는 이어지지 않으므로 메타에 명시적 매핑 배열(`dir_pages`)이 필요하다.
- **왜 디렉터리 핀을 놓고 버킷을 잡나**: 디렉터리 페이지를 쥔 채 버킷을 fetch하면 프레임 1개 풀에서 miss → `take_frame`이 빈 프레임도, 핀 0인 프레임도 못 찾아 `PoolError::AllPinned`(`src/storage/pool.rs:249-252`). 버킷 번호(`u64`)만 복사해 나오면 동시 핀 1개 → 프레임 1개에서도 조회 가능(테스트 `get_needs_only_one_frame`: fetch 2회).

### 오답 노트

| 내 답 | 실제 |
|---|---|
| "slot 3000 → 첫번째칸" / "3000/2048 = 1 → 페이지 1" | 1은 `dir_pages` 배열의 **자리 번호**. 페이지 번호는 그 자리에 **저장된 값**(`dir_pages[1]` = 5). 메타 → 디렉터리 → 버킷, 간접 참조가 2단 |

## 3. 변이

| 변이 | 실패한 테스트 수 |
|---|---|
| 디렉터리 페이지를 항상 0번 자리로 | 1 |
| 칸을 `slot / DIR_SLOTS`로 | 2 |
| mask에서 `- 1` 빼기 | 2 |
| 디렉터리 핀을 쥔 채 버킷 fetch | 1 (프레임 1개 `AllPinned`) |

전체 73 + 33 + 31 green · clippy · fmt 통과.

## 4. 진행

| diff | 게이트 |
|---|---|
| 09 | Q1 ✅ (사전 점검 2회 — 자리/값 혼동) · Q2 ✅ |

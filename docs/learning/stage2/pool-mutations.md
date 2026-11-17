# 2단계 — 버퍼풀 변이 × 전체 스위트 (diff 12)

## 1. 범위

코드 변경 없음. `src/storage/pool.rs`의 `take_frame`에 변이 3종을 넣고 **전체 스위트**(lib · buffer_pool · paged_btree · paged_hash)를 돌렸다.
목적: 인덱스 테스트가 버퍼풀 버그를 얼마나 잡는가.

`src/storage/pool.rs:245-264`

```rust
fn take_frame(&self, ledger: &mut Ledger) -> Result<usize> {
    if let Some(free) = ledger.meta.iter().position(|m| m.page.is_none()) {
        return Ok(free);
    }
    let victim = (0..ledger.meta.len())
        .filter(|&f| ledger.meta[f].pins == 0)        // (c) 이 줄 삭제
        .min_by_key(|&f| ledger.meta[f].last_used)    // (b) max_by_key로
        .ok_or(PoolError::AllPinned)?;
    let old = ledger.meta[victim];
    let old_id = old.page.expect("non-free frame holds a page");
    if old.dirty {                                    // (a) 쓰기 생략
        self.pager.borrow_mut().write(old_id, &self.frames[victim].borrow())?;
    }
    ...
}
```

## 2. 결과

| 변이 | lib | buffer_pool | paged_btree | paged_hash |
|---|---|---|---|---|
| (a) 내보낼 때 dirty 쓰기 생략 | 16 실패 | 10 실패 | 21 실패 | 31 실패 |
| (b) LRU 대신 MRU | 1 실패 | 7 실패 | **생존** | **생존** |
| (c) 핀 무시 | 1 실패 | 6 실패 | **생존** | **생존** |

버퍼풀 버그는 버퍼풀 테스트가 3종 모두 잡는다. 인덱스 테스트는 (a)만 잡는다.

## 3. 왜 (a)만 — 결과 vs 카운터 (게이트 Q1 pass, 사전 점검 1회)

- 인덱스 테스트는 `get`·`range` **결과**를 `BTreeMap`과 비교한다. 카운터(hit/miss/io)는 보지 않는다.
- (a): 수정한 페이지가 디스크에 안 남는다 → 다시 읽으면 옛 내용 → **결과가 틀린다**.
- (b): MRU도 dirty면 먼저 쓴다 → 데이터는 그대로, **어느 프레임을 내보내느냐만** 바뀐다 → hit/miss/io 수만 달라진다(miss는 보통 늘지만 항상은 아님).
- 즉 (b)·(c)는 인덱스 테스트의 **커버리지 공백**이고, 그 공백은 pool 단위 테스트가 메운다.

## 4. 핀 거름은 언제 필요한가 (게이트 Q2 pass, 출제 전제 정정 1회)

### split 경로 — 필터 없어도 안 터진다

`src/index/paged_hash.rs` `split`

```rust
let mut old_page = self.pool.fetch_mut(id)?;          // ① 핀 + last_used = 가장 큰 tick
let (new_id, mut new_page) = self.pool.new_page()?;   // ② 곧바로 take_frame
```

핀을 잡자마자 `take_frame` → 핀된 프레임은 **늘 가장 최근** → LRU가 애초에 고르지 않는다. 필터가 하는 일을 LRU 순서가 우연히 대신한다.

### 핀을 쥔 채 다른 페이지에 접근하면 — 필터가 꼭 필요

프레임 2개:

```rust
let a = pool.fetch(A)?;      // tick 1, 핀 유지
{ let _b = pool.fetch(B)?; } // tick 2, 바로 놓음
let c = pool.fetch(C)?;      // miss → take_frame
```

| | 고르는 프레임 | 이유 |
|---|---|---|
| 원래 | B | A는 핀 1 → 필터에서 빠짐 |
| (c) 변이 | A | tick 1이 가장 오래됨 → LRU가 핀된 A를 고름 |

(c)에서 실제 실행 결과(스크래치 테스트):

```
panicked at src/storage/pool.rs:232:47:
RefCell already borrowed
```

`a`가 그 프레임의 `Ref`(`PageRef.page`, pool.rs:269-273)를 쥐고 있어 `pin`이 C를 읽어 넣는 `borrow_mut()`에서 panic. RefCell 없는 풀(C/C++)이라면 `a`가 **조용히 C 내용으로 바뀐다** — 더 무서운 silent failure.

**결론: 핀 여부와 최근 사용 순서는 서로 독립이다.** 핀을 오래 쥘수록 그 프레임은 늙는다. LRU만으로는 핀을 보호할 수 없어서 필터가 일반적으로 필요하다.

### 오답 노트

| 내 답 | 실제 |
|---|---|
| "a에 C 내용이 보여" | 원리상 맞지만 이 코드에선 RefCell이 막아 panic. 질문이 "무엇이 보이나"로 잘못된 전제를 줬다(튜터 정정 — 시도 미산정) |
| "miss가 늘어" | 보통 그렇지만 핵심은 "결과는 같고 카운터만 변한다" |

## 5. 진행

| diff | 게이트 |
|---|---|
| 12 | Q1 ✅ (사전 점검 1회, 힌트 무거움) · Q2 ✅ (출제 전제 정정 후 재질문) |

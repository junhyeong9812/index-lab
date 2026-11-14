# 2단계 — 버퍼풀 (diff 03–04)

## 1. 구조

```rust
pub struct BufferPool<S: Storage> {
    pager: RefCell<Pager<S>>,
    frames: Vec<RefCell<Box<Page>>>,   // 프레임 내용 — 가드가 빌린다
    ledger: RefCell<Ledger>,           // 페이지 테이블·핀·dirty·last_used·통계
}
```

- `fetch(id)` → 읽기 가드 `PageRef` · `fetch_mut(id)` → 쓰기 가드 `PageMut` · drop되면 `Drop`에서 언핀.
- `PageMut`는 `DerefMut`(실제로 `&mut`로 건드림)일 때만 dirty — 읽기만 하면 clean.
- `new_page()` — 번호 할당 + 0 프레임, 디스크 읽기 없음, dirty.
- `flush()` — dirty 전부 쓰고 clean으로.

## 2. 프레임 내주기 `take_frame`

① 빈 프레임 → ② 핀 0만 후보 → ③ `last_used` 최소(LRU) → 없으면 `AllPinned` → ④ dirty면 **먼저** 디스크에 쓰기 → 장부에서 지우고 반환. 새 페이지를 읽어 넣는 건 그다음.

## 3. 세 장치가 각각 막는 것

| 장치 | 없으면 | 변이 실측 |
|---|---|---|
| 핀 (`pins == 0` 필터) | 가드가 쥔 프레임이 내보내져, 가드가 다른 페이지를 보게 됨 | `RefCell` 패닉 → 소멸자 중 패닉 abort (우연한 2차 안전망) |
| dirty write-back (`if old.dirty`) | 고친 값이 디스크에 안 감 → 다시 fetch하면 옛값, **오류 없이 조용히 소실** | 테스트 1개 실패 |
| `if` 조건 (dirty일 때만) | 무조건 쓰면 결과는 맞지만 clean 페이지도 써서 write 낭비·측정 부풀림 | — |
| LRU (`min_by_key`) | `max`면 방금 쓴(곧 또 쓸) 페이지를 내보내 miss 증가 | 테스트 1개 실패 |

### dirty 시간표 (페이지 4, 5 → 9)

| 순서 | 일 | 프레임 X | 디스크 4 |
|---|---|---|---|
| ① | `fetch(4)` | 4번, 5 | 5 |
| ② | `fetch_mut`로 9로 고침 → dirty | 4번, 9 | 5 |
| ③ | 4가 LRU 희생자 → dirty라 **프레임 내용(9)을 디스크에 씀** | 4번, 9 | **9** |
| ④ | 프레임 X에 페이지 7 적재 | 7번 | 9 |
| ⑤ | `fetch(4)` | | → 9 |

- if 블록이 없으면 ③에서 안 씀 → 디스크 5 → ⑤에서 5, 9는 사라짐.
- 항상 쓰면 clean(5)일 때도 같은 5를 씀 → 결과는 맞고 write만 늘어남.

## 4. 저장소 계약 변경 (diff 03에서 발견)

버퍼풀은 할당 순서와 다른 순서로 내보낸다(새 페이지 6이 5보다 먼저 쓰일 수 있음). 01·02의 "구멍 금지"와 충돌 → "끝 뒤 쓰기 = 성장, 사이 페이지는 0"으로 변경. 할당 안 된 번호 쓰기는 Pager(`id < next`)가 계속 막는다. 대안(할당 즉시 0 페이지 쓰기)은 분할마다 write +1로 측정이 왜곡돼 기각.

## 4-2. 블라인드 테스트와 Drop 안 패닉 (diff 04)

블라인드 워커(구현 비열람)가 `tests/buffer_pool.rs`(예제 31 + 참조 모델 proptest 2)를 짰다. 실코드 변이: dirty 쓰기 생략 10 실패 · MRU 7 실패 · 핀 무시 → **프로세스 abort**.

abort 경로:
1. 핀 무시 → 핀된 프레임이 희생자 → `ledger.meta[victim] = FrameMeta::default()` (pins 0, pool.rs:262)
2. 새 페이지 적재 중 `RefCell` 패닉(가드가 프레임을 빌리고 있음) — **1차 패닉**, 되감기 시작
3. 되감기가 살아 있던 가드를 drop → `unpin` → `debug_assert!(m.pins > 0)` 실패 — **2차 패닉**
4. 되감는 중 또 패닉 → Rust는 abort

| | panic (되감기) | abort |
|---|---|---|
| 동작 | 호출 스택을 거슬러 오르며 살아 있는 값의 `Drop` 실행 | 프로세스 즉시 종료(SIGABRT) |
| `catch_unwind` | 잡힌다 — `cargo test`가 테스트마다 감싸서 그 테스트만 FAILED | 못 잡는다 |
| 나머지 테스트 | 계속 돈다 | 함께 죽는다, 결과 요약 줄도 없음 |

실측(데모): `assert_eq!(1, 2)` 테스트는 FAILED 후 다음으로. `Drop` 안에서 패닉하는 값을 든 채 패닉한 테스트는 `panic in a destructor during cleanup … aborting` → SIGABRT, 뒤 테스트 미실행.

왜 두 번째 패닉이면 abort인가: 되감기는 패닉 **하나**를 위로 운반하는 장치다. 운반 도중 두 번째가 생기면 둘을 동시에 처리할 방법이 없어 Rust는 포기한다. → **Drop 안에서는 패닉하지 마라.**

수정(pool.rs:73-83):
```rust
debug_assert!(m.pins > 0 || std::thread::panicking(), "unpin of an unpinned frame");
m.pins = m.pins.saturating_sub(1);
```
- 평소(`panicking() == false`): 검사 **켜짐** — 핀 1번에 언핀 2번 같은 장부 버그를 잡는다.
- 되감는 중(`panicking() == true`): 검사 **꺼짐** — 1차 패닉이 그대로 전파되어 `catch_unwind`가 잡을 수 있게.
- `saturating_sub`: 그냥 `-=`면 `u32` 0−1이 debug에선 오버플로 패닉(또 Drop 안 패닉), release에선 `u32::MAX`로 감겨 그 프레임이 영원히 핀 상태(내보내기 불가). 0에서 멈춰 둘 다 피한다.

## 5. 진행

| diff | 게이트 |
|---|---|
| 03 | 1/2 fail(Q2) → 2/2 pass |
| 04 | 1/2 fail → 2/2 fail → 설명 후 통과 |

## 6. 오답 노트

- **Q2 "롤백"**: 밀려날 때 되돌리는 동작은 없다. `pager.write(old_id, 프레임)`은 프레임에 있는 **고친 내용**을 디스크로 보낸다. 쓰기(③)가 새 페이지 적재(④)보다 먼저라 그 순간 프레임엔 아직 페이지 4가 있다.
- **Q2 "불일치"**: 프레임 ≠ 디스크인 상태는 dirty의 정의라 그 자체는 정상. 문제는 쓰지 않고 프레임을 **비운 뒤** — 그때 고친 값이 사라진다.
- **Q2 첫 답 혼동**: "없으면"(값 소실)과 "항상"(값 정상·I/O 낭비) 두 경우를 섞음 → 따로 본다.
- **04 Q1 "스레드가 잡고 있다"**: 스레드는 하나뿐. 잡고 있는 건 **가드**이고, 문제는 그 가드가 drop될 때 장부의 `pins`가 이미 0(`default()`)이라는 것.
- **04 panic vs abort**: panic은 되감기(Drop 실행, `catch_unwind`로 멈춤 가능), abort는 즉시 종료(잡을 수 없음). 되감는 중 2차 패닉 = abort.
- **04 `|| panicking()`**: 검사를 지운 게 아니라 되감는 중에만 끈다. 평소엔 이중 언핀 버그를 잡는다.

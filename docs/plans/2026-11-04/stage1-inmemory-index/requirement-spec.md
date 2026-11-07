# 요구사항 명세서 — 로드맵 1단계: 메모리 내 B+tree · Hash + 속성 테스트

> 작성일: 2026-11-04 · 작업 폴더: `docs/plans/2026-11-04/stage1-inmemory-index/` · 모드: **lazy**(매 diff 이해 게이트)

## 0. 요구사항 원문 (인터뷰 기록)

- 원문: "이 프로젝트부터 같이 하나씩 진행보자" → "레이지모드로 하나씩 가보자"
- 기준: README 로드맵 1 "메모리 내 B+tree, Hash 구현 및 속성 테스트". 키 `u64`, 값 고정 길이, `trait Index { insert, get, range }`.
- 트레이드오프 선택: 아래 §10 표 — **사용자가 직접 선택해 채운다**.

## 1. 목표·대상 (필수)

`index-lab` 저장소에 Rust 라이브러리 크레이트를 만들고, 메모리 안에서 동작하는 B+tree와 Extendible Hash를 `Index` trait 뒤에 구현한다. 두 구현 모두 `BTreeMap`을 정답지로 한 속성 테스트를 통과한다. (페이지·버퍼풀은 2단계 — 이번 범위 밖.)

## 2. 경계·불변식 (필수)

- **I1 정답 일치**: 임의의 삽입 순서열 뒤에 `get`·`range`·`range_rev` 결과가 `BTreeMap`과 같다(두 구현 모두, `range_rev` = `BTreeMap::range(..).rev()`).
- **I2 B+tree 구조**: 모든 리프 깊이 같음 · 노드 안 키 정렬 · 루트 외 노드 키 수 ≥ ⌈max/2⌉ − 1 수준(분할 규칙상 하한) · 리프 `next` 연결이 모든 키를 오름차순으로, `prev` 연결이 내림차순으로 한 번씩 지남 · `a.next = b` ⇔ `b.prev = a` · 내부 노드 구분자 규칙(왼쪽 < 구분자 ≤ 오른쪽).
- **I3 Hash 구조**: 모든 버킷 지역 깊이 ≤ 전역 깊이 · 디렉토리 길이 = 2^전역 깊이 · 지역 깊이 d인 버킷을 가리키는 디렉토리 칸 수 = 2^(전역 − d) · 버킷 안 키는 모두 그 버킷의 하위 d비트 접미사를 공유.
- **I4 갱신 의미**: 같은 키 재삽입은 값 교체(upsert), 이전 값 `Some(old)` 반환(T3).

## 3. 기준소스 (필수)

README(이 저장소) · Fagin 외 1979 "Extendible Hashing" · B+tree 표준 분할 규칙(리프는 구분자 copy-up, 내부는 push-up) · `std::collections::BTreeMap`(정답지).

## 4. 금지영역 (필수)

- 페이지(Pager)·버퍼풀·디스크 I/O·벤치마크 하네스 — 2·3단계.
- README 본문 수정 금지.
- 런타임 의존성 추가 금지(이번 단계는 dev-dependency `proptest`만).

## 5. 검증 방법 (필수)

- `cargo test` — 단위 테스트 + 속성 테스트(I1~I3) green.
- `cargo clippy -- -D warnings`, `cargo fmt --check`.
- 변이 점검 1회씩: B+tree 분할에서 구분자 규칙을 틀리게 / Hash 분할에서 디렉토리 칸 재지정을 틀리게 하면 속성 테스트가 잡는지.
- lazy 게이트: diff마다 사용자 주관식 이해 판정(워커) 통과.

## 6. stakes (필수)

- 판정: **낮음** — 신규 학습 저장소, 외부 영향 없음, 테스트가 정답지와 대조.

## 7. 자율성

- [ ] auto
- [x] lazy (사용자 선택 2026-11-04)

## 8. load-bearing 가정

- A1: proptest 1.12.0이 현재 툴체인(rustc 1.92.0, proptest MSRV 1.88)에서 빌드된다. → task 01 직후 `cargo test`로 확인.

## 9. 리서치 (lazy §2 — 계획 전 필수)

### 내부
- 저장소는 README·docs·.gitignore뿐(커밋 4개). 코드 없음 — 재사용할 것 없음.
- README 구조안: `src/index/{mod.rs, btree.rs, hash.rs}`, trait `Index { insert, get, range }`.
- 환경: rustc/cargo 1.92.0, proptest 최신 1.12.0(rust-version 1.88), proptest-state-machine 0.9.0 (`cargo search`·`cargo info`, 2026-11-04).

### 외부
- **Extendible hashing** — 디렉토리 길이 = 2^전역 깊이, 버킷마다 지역 깊이(≤ 전역). 넘친 버킷의 지역 깊이 < 전역이면 디렉토리 크기 그대로 두고 그 버킷을 가리키던 2^(전역−지역)칸 중 절반을 새 버킷으로 재지정, 같으면 디렉토리를 두 배로 복사한 뒤 분할. 분할 후 새로 드러난 비트로 키를 재배치. 출처: [Wikipedia "Extendible hashing"](https://en.wikipedia.org/wiki/Extendible_hashing) (Fagin 외 1979 인용), [Duke CompSci 516 Lecture 7](https://courses.cs.duke.edu/spring16/compsci516/Lectures/Lecture-7.pdf) ("directory is doubled by copying it over and fixing pointer to split image page"), [BU CS460 hashing](https://www.cs.bu.edu/fac/gkollios/cs460f17/Slides/hashing.pdf).
  - 함정: 키의 하위 비트가 같은 키들이 버킷 용량보다 많으면 분할해도 갈라지지 않아 **디렉토리가 끝없이 두 배**가 된다(예: 해시 = 키 그대로이고 키가 전부 2^k의 배수). → 해시 함수 선택(§10 T8)과 최대 깊이 가드가 필요.
- **B+tree** — 리프 분할은 가운데 키를 부모로 **복사(copy-up)**, 내부 노드 분할은 가운데 키를 **올림(push-up)**. 루트 분할이 높이를 1 늘린다. 비용 ≈ log_F N(F = fan-out). 루트 외 최소 50% 점유. 출처: [Buffalo CSE350 B-tree overview](https://cse.buffalo.edu/adblab/people/zzhao35/teaching/cse350_spring26/files/04-btree-overview.pdf), [CSU CS165 B+Trees](https://www.cs.colostate.edu/~cs165/.Spring20/slides/15-BplusTrees.pdf), [UC Davis ECS165B Lecture 9](https://www.cs.ucdavis.edu/~green/courses/ecs165b/Lecture9.pdf).
- **속성 테스트** — 실무 Rust 크레이트들이 `BTreeMap`을 정답지(reference model)로 두고 무작위 연산열을 실제 구조와 나란히 적용해 매 연산 결과를 대조. 방식은 (a) `proptest-state-machine`(`ReferenceStateMachine`·`StateMachineTest`) 또는 (b) 평범한 proptest로 `Vec<Op>`를 생성. 출처: [arctic-wt state_machine.rs](https://docs.rs/crate/arctic-wt/latest/source/tests/state_machine.rs), [ic-stable-structures btreemap proptests](https://docs.rs/crate/ic-stable-structures/0.6.9/source/src/btreemap/proptests.rs), [holt properties.rs](https://docs.rs/crate/holt/latest/source/tests/properties.rs).

## 10. 트레이드오프 — 사용자 선택

| # | 결정 | 후보 | 선택 |
|---|---|---|---|
| T1 | `range` 반환 형태 | A `Vec<(u64, V)>` 반환 — 단순, 2단계(버퍼풀 빌림)와 충돌 없음 · B `impl Iterator` — 지연 평가·메모리 적음, 2단계에서 버퍼풀 빌림 수명과 얽힘 · C 콜백 `range(lo, hi, f)` — 할당 없음, 사용성 낮음 | **A** `Vec<(u64, V)>` |
| T2 | 값 타입 | A `u64` 고정 — 이번 단계 단순 · B `[u8; N]` 상수 제네릭 — "고정 길이 값"을 처음부터, 페이지 레이아웃(2단계)과 맞음 | **A** `u64` |
| T3 | 같은 키 재삽입 | A upsert(교체, 이전 값 반환 `Option<V>`) — `BTreeMap`과 같음 · B 거부(`Err(Duplicate)`) — 유일 인덱스 의미 | **A** upsert |
| T4 | 삭제 | A 이번 단계 없음 — README trait에 없음, 범위 축소 · B `remove` 포함 — 병합·재분배(B+tree)·버킷 합치기(Hash)까지 커짐 | **A** 없음 |
| T5 | B+tree 노드 표현 | A arena `Vec<Node>` + 정수 `NodeId` — 2단계에서 `PageId`로 바꾸기 쉬움, 소유권 단순 · B `Box`/`Rc<RefCell>` 포인터 트리 — 교과서적, 2단계에서 갈아엎어야 함 | **A** arena + `NodeId` (사용자 질문: "포인터 트리가 인덱스에 더 가깝지 않나?" → 디스크 B+tree는 페이지 번호로 연결 — InnoDB node pointer·형제 링크 4바이트 페이지 번호, PostgreSQL btpo_prev/next 블록 번호. NodeId = 2단계 PageId) |
| T6 | fan-out 지정 | A 생성자 인자 `BTree::new(max_keys)` — 테스트는 작은 값(4)으로 분할을 강제, 실험은 큰 값 · B 상수 고정 | **A** `BTree::new(max_keys)` |
| T7 | 리프 연결 | A 단방향 `next` — 범위 조회에 충분 · B 양방향 — 역순 범위용, 지금은 불필요 | **리프 양방향 + 역방향 range** (사용자 질문: "탐색 전체를 보려면 양방향도?" → InnoDB FIL_PAGE_PREV/NEXT·PostgreSQL btpo_prev/next 모두 양방향, 역방향 스캔에 사용. 분할 시 오른쪽 형제 prev 갱신 = 2단계에서 페이지 쓰기 +1로 측정 가능) |
| T8 | Hash 함수 | A 키 그대로(하위 비트) — 단조 증가 키에서 완벽 분산, 구조적 키에서 디렉토리 폭주 위험 · B 비트 섞기(splitmix64 finalizer, 전단사) — 분포 무관 고른 분산, 서로 다른 키는 해시도 다름 → 무한 두 배 불가 | **B** splitmix64 finalizer |
| T9 | Hash 버킷 용량 | A 생성자 인자 `Hash::new(bucket_cap)` — 테스트는 2~4로 두 배를 강제 · B 상수 고정 | **A** `Hash::new(bucket_cap)` |
| T10 | Hash `range` | A 전체 버킷 스캔 후 필터·정렬 — 결과는 맞고 "해시는 범위에 약하다"를 3단계에서 그대로 측정 · B 미지원(`Unsupported`) — 실험 격자에서 Hash 범위 칸이 비게 됨 | **A** 전체 스캔 후 필터·정렬 |
| T11 | 속성 테스트 방식 | A 평범한 proptest + `Vec<Op>` 생성 — 의존성 1개, 읽기 쉬움 · B `proptest-state-machine` — 상태 기계 프레임워크, 축소(shrink)가 연산 단위로 정교, 의존성 +1·학습 비용 | **A** proptest + `Vec<Op>` |
| T12 | 구조 관찰용 공개 API | A `depth()`(B+tree)·`global_depth()`/`bucket_count()`(Hash) + 불변식 검사 `check()` 공개 — 테스트와 3단계 실험이 같이 씀 · B 테스트 전용(`#[cfg(test)]`)만 | **A** 공개 (`depth`·`global_depth`·`bucket_count`·`check`) |
| T13 | `range_rev` 위치 | A `Index` trait에 추가 — Hash도 전체 스캔 후 내림차순, 실험 격자에 역순 범위 축 · B B+tree 전용 메서드 | **A** trait에 추가 |

## 11. 디테일 계획 — diff 단위 (각 diff = lazy 이해 게이트 1회)

| diff | 내용 | 의존 |
|---|---|---|
| 01 | `cargo init --lib` + `Cargo.toml`(dev-dep proptest) + `src/index/mod.rs` `Index` trait(`insert`·`get`·`range`·`range_rev`) (+ A1 스모크) | — |
| 02 | B+tree 노드 구조체 + `get`(루트→리프 탐색) | 01 |
| 03 | B+tree `insert` — 리프 삽입·리프 분할(copy-up) + 형제 `prev`·`next` 재연결(오른쪽 형제 `prev` 갱신 포함) | 02 |
| 04 | B+tree `insert` — 내부 분할(push-up)·루트 분할(높이 +1) | 03 |
| 05 | B+tree `range`(시작 리프 → `next`) · `range_rev`(끝 리프 → `prev`) | 04 |
| 06 | B+tree 테스트 — 단위(작은 fan-out 분할 시나리오) + 속성(정답지 대조·I2 검사) | 05 |
| 07 | Hash 디렉토리·버킷 구조체 + 해시 함수 + `get` | 01 |
| 08 | Hash `insert` — 지역 < 전역 분할 · 지역 = 전역 디렉토리 두 배 | 07 |
| 09 | Hash `range`·`range_rev` + 테스트 — 단위(두 배 시나리오) + 속성(정답지 대조·I3 검사) | 08 |
| 10 | 변이 점검 2건 + clippy·fmt + 완료 요약 | 06, 09 |

## 12. study-note 기록 위치 — 사용자 선택

- **`study-note/lab/storage-labs/index-lab/`** (사용자 선택) — backend-labs 방식, 단계별 append. 1단계 완료 시 작성.

## 승인 상태

- [x] 필수 6칸 전부 기입
- [x] 트레이드오프 T1~T13 사용자 선택 (2026-11-04)
- [x] 계획 이해 게이트 통과 (2026-11-08, 독립 판정 Q1·Q2·Q3 PASS — 보완 권고: 내부 노드도 구분자 키를 가짐, copy-up은 키가 리프에 남고 push-up은 빠짐)
- [x] 사용자 합의 → SPEC=1 (2026-11-08)
- [x] 자율성 선택 → lazy

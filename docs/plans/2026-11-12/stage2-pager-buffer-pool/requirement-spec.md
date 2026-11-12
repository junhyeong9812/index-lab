# 요구사항 명세서 — 로드맵 2단계: Pager + 버퍼풀, 인덱스를 페이지 위로

> 작성일: 2026-11-12 · 작업 폴더: `docs/plans/2026-11-12/stage2-pager-buffer-pool/` · 브랜치: `feat/stage2-pager-buffer-pool` (main 8cf93b9에서 분기)

## 0. 요구사항 원문 (인터뷰 기록)

- 원문: "2단계 진행하자" (1단계 마감 직후)
- README에서 이미 정해진 것(재질문 안 함): 페이지 16KB 고정 · 고정 프레임 수 + LRU 버퍼풀(크기 조절 가능) · `std::fs` + `read_at`/`write_at`(mmap 회피) · I/O 횟수는 Pager 내장 카운터 · 페이지 ↔ 구조체는 `bytemuck` · 범위 밖: 트랜잭션·WAL·복구·동시성·가변 길이 레코드.
- Q/A (2026-11-12, 모두 권장안 선택):
  - Q1 1단계 메모리 구현 → **남기고 페이지 버전을 새로 추가**(`Index` trait 구현 4개).
  - Q2 페이지 접근 → **핀 가드(RAII)** — 가드가 살아 있는 동안 pin, drop 시 unpin.
  - Q3 Hash 디렉터리 → **페이지에 저장**(두 배 쓰기가 카운터에 잡히게 — 4단계 실험 대비).
  - Q4 테스트 저장소 → **둘 다**: 메모리 가짜 디스크(proptest) + 실제 파일(재열기 스모크), dev-dep `tempfile`.
  - Q5 쓰기 정책 → **write-back**(내보낼 때·`flush()` 때만 쓰기).
  - Q6 오류 → **Pager·버퍼풀은 `Result`, 페이지 인덱스의 `Index` 구현에서만 `expect`로 panic**. trait·1단계 구현 무변경.
  - Q7 영속성 → **지원 — 페이지 0 = 메타 페이지**, `flush` 후 다시 열면 같은 내용. 비정상 종료 안전성은 범위 밖(WAL 없음).
  - Q8 바이트 변환 → **`bytemuck` 추가**(`#[repr(C)]` + `Pod`).
  - Q9 가드 여러 개 동시 → **`&self` API + 프레임마다 `RefCell`**(단일 스레드 전제). 같은 페이지 이중 가변 빌림은 런타임에 잡힘.
  - Q10 카운터 → **Pager 실 I/O(read·write) + 버퍼풀 hit·miss·eviction**.
  - Q11 순서 → **아래부터 쌓기**: Pager → 버퍼풀 → 페이지 B+tree → 페이지 Hash.

## 1. 목표·대상 (필수)

`index-lab`에 16KB 페이지 단위 Pager(파일·메모리 두 저장소, read/write 카운터)와 LRU·핀·write-back 버퍼풀을 만들고, 그 위에서 동작하는 **페이지 B+tree·페이지 Extendible Hash**를 `Index` trait 뒤에 추가한다. 두 페이지 구현은 버퍼풀이 작아 내보내기가 계속 일어나도 `BTreeMap` 정답지와 같은 결과를 내고, `flush` 후 다시 열면 같은 내용을 보인다.

## 2. 경계·불변식 (필수)

- **P1 Pager 계수**: read 카운터 = 디스크에서 실제로 읽은 페이지 수, write 카운터 = 실제로 쓴 페이지 수. 파일 저장소와 메모리 저장소의 의미가 같다(같은 연산열 → 같은 내용·같은 카운트).
- **P2 버퍼풀**:
  - 핀이 1 이상인 페이지는 내보내지 않는다.
  - 페이지 테이블(PageId → 프레임)과 프레임 내용이 일치하고, 한 페이지는 최대 한 프레임에만 있다. 사용 프레임 수 ≤ 용량.
  - dirty 프레임은 재사용 전에 반드시 디스크에 쓴다(쓰기 누락 = 데이터 손실). 깨끗한 프레임은 쓰지 않는다.
  - 내보낼 대상 = 핀 0인 프레임 중 가장 오래전에 쓰인 것(LRU).
  - 모든 프레임이 핀이면 `fetch`는 panic이 아니라 `Err`.
  - `flush()` 후 dirty 프레임 0.
  - 적중이면 디스크 read 0회.
- **P3 정답 일치**: 페이지 B+tree·페이지 Hash가 1단계 I1(get·range·range_rev = `BTreeMap`)과 I4(upsert)를 만족한다. 버퍼풀 크기와 무관(최소 프레임 수 이상이면).
- **P4 구조 불변식**: 페이지 B+tree는 1단계 I2(리프 깊이 동일·키 정렬·구분자 `왼쪽 < 구분자 ≤ 오른쪽`·하한·next/prev 대칭), 페이지 Hash는 I3(디렉터리 길이 2^전역·칸 수 2^(전역−지역)·키의 `mix` 접미사 일치)을 만족한다. 각자 `check()`로 검사.
- **P5 영속성**: `flush` → 닫기 → 다시 열기 후 모든 키·구조 메타(루트·높이·전역 깊이·페이지 수)가 같다.
- **P6 핀 상한**: 인덱스 연산 하나가 동시에 쥐는 핀 수에 상수 상한이 있다(트리 높이에 비례하지 않음 — 조상 페이지는 쥐고 내려가지 않는다). 이 상한이 버퍼풀 최소 프레임 수다.

## 3. 기준소스 (필수)

README(이 저장소 — 범위·기술 스택·구조) · 1단계 명세의 I1~I4와 `Index` trait(`src/index/mod.rs`) · `std::collections::BTreeMap`(정답지) · 1단계 메모리 구현(`btree.rs`·`hash.rs` — 알고리즘 기준) · 버퍼풀: CMU 15-445 BusTub 설계(핀 수·dirty·replacer·RAII page guard) · 디스크 Extendible Hash 페이지 구성: 15-445 프로젝트 노트(header → directory → bucket 페이지).

## 4. 금지영역 (필수)

- `Index` trait 시그니처 변경 금지 · 1단계 `btree.rs`·`hash.rs` 동작 변경 금지(필요하면 공유 함수 추출만, 테스트 38개 무변경 green).
- README 본문 수정 금지.
- 트랜잭션·WAL·동시성(멀티스레드)·벤치마크 하네스(3단계)·삭제 연산 — 범위 밖.
- 런타임 의존성은 `bytemuck`만 추가, dev-dependency는 `tempfile`만 추가. `unsafe` 코드 금지(바이트 변환은 `bytemuck`).

## 5. 검증 방법 (필수)

- `cargo test` — 1단계 38개 + 2단계 단위·속성 테스트 green. `cargo clippy --all-targets -- -D warnings`, `cargo fmt --check`.
- 속성 테스트: 페이지 인덱스 × 작은 `max_keys`/버킷 용량 × **작은 버퍼풀**(내보내기 강제) × `BTreeMap` 대조 + 매 삽입 `check()` + 핀 누수 0(연산 후 전체 핀 0).
- 버퍼풀 단위: LRU 순서·핀 페이지 비내보내기·dirty 쓰기·전부 핀이면 `Err`·hit/miss 카운트를 정확한 숫자로 검증.
- 영속성: 실제 파일(tempfile)에서 삽입 → `flush` → 닫기 → 다시 열기 → 정답지 대조.
- 변이 점검 1회씩: (a) 내보낼 때 dirty 쓰기 생략 (b) LRU 대신 최근 것 내보내기 (c) 핀 무시 내보내기 — 테스트가 잡는지.
- 테스트 설계는 블라인드 워커(구현 본문 미열람, spec·타입·시그니처만) — 1단계와 같은 방식.
- lazy 게이트: diff마다 사용자 주관식 이해 판정(독립 Opus 워커) 통과. 사전 점검 후 판정(메모리 lazy-gate-precheck).

## 6. stakes (필수)

- 판정: **낮음** — 신규 학습 저장소, 외부 영향·불가역 데이터 없음, 정답지 대조 테스트가 막아 줌. 단 버퍼풀·`RefCell` 가드는 낯선 영역이라 테스트는 블라인드 워커로 분리(1단계와 동일 수준 유지).

---

## 7. 자율성 (모드 — stakes와 별개 축)

- [ ] auto
- [x] lazy (사용자 선택 2026-11-12)

## 8. load-bearing 가정

- **A1 `bytemuck` 배열**: 16KB짜리 `#[repr(C)]` 페이지 구조체(예: `[u64; 1022]` 배열 포함)에 `Pod`·`Zeroable` derive가 되고, `&[u8; 16384]` ↔ `&Struct` 캐스팅이 패딩 없이 된다. → diff 01 직후 스모크(긴 배열의 `Pod` 구현 여부·크기 단언 `const _: () = assert!(size_of::<P>() == PAGE_SIZE)`).
- **A2 핀 상한**: `RefCell` 프레임 + `&self` 가드로 페이지 B+tree 삽입(리프 분할 + 오른쪽 형제 `prev` 갱신 + 부모 갱신)을 **동시 핀 ≤ 4**로 할 수 있다(조상 페이지는 재조회). → 버퍼풀 diff 직후 스모크(용량 4 풀에서 높이 5 이상 트리 삽입).

## 9. 리서치

### 내부
- 1단계 산출물: `Index` trait(dyn 호환 가드 `mod.rs:25-26`), `btree.rs`(arena `Vec<Node>` + `NodeId` — T5에서 2단계 `PageId`를 예정), `hash.rs`(directory `Vec<BucketId>` + buckets, `mix`=splitmix64), 테스트 38개(1.8s).
- 1단계 NEXT 관측 신호: Hash 분할 칸 재지정이 디렉터리 전체 훑기(O(2^전역)) — 디렉터리가 페이지에 오르면 분할 1회 = 디렉터리 페이지 전부 읽기·쓰기. `range`는 `Vec` 반환(T1 유지).
- 환경: rustc 1.92.0 · `bytemuck` 1.25.2 · `tempfile` 3.27.0(rust-version 1.63) (`cargo search`·`cargo info`, 2026-11-12).

### 외부
- **버퍼풀** — 핀 수가 0이 아닌 페이지는 내보내지 않고, 내보내거나 flush할 때 dirty면 먼저 디스크에 쓴다. replacer는 핀 페이지를 "내보낼 수 없음"으로 표시해 건너뛴다. page guard = 핀·언핀을 관리하는 RAII 래퍼. 출처: [BusTub BPM 구현 노트](https://peteryaonyu.github.io/2023/12/18/Database-Bustub-Buffer-Pool-Manager-Implementation), [CMU 15-445 Fall 2023 Project 1 노트](https://sszgwdk.github.io/posts/cmu445_p1), [15-445 노트](https://quartz-4uj.pages.dev/Notes/CMU-15-445), [LRU와 LRU-K](https://jungle430.github.io/posts/cmu-15-445-database-systems/lru_and_lru-k).
- **디스크 Extendible Hash** — header 페이지(디렉터리 페이지 번호 배열·최대 깊이) → directory 페이지(전역 깊이·지역 깊이·버킷 페이지 번호) → bucket 페이지(키·값 배열) 3단. 디렉터리는 파일보다 훨씬 작아 두 배가 싸고, 분할은 데이터 페이지 하나만 건드린다. 출처: [15-445 Extendible Hash Index 노트](https://aniurm.github.io/notebook/course_notes/cmu15445/hash-index), [Duke CompSci 516 Lecture 10](https://www.cs.duke.edu/courses/fall19/compsci516/Lectures/Lecture-10-tree-hash-index.pdf).

## 10. 트레이드오프 — 인터뷰 밖 세부(제안 — 이의 있으면 합의 때 변경)

| # | 결정 | 제안 | 이유 |
|---|---|---|---|
| S1 | `PageId` 타입 | `u64` | 페이지 구조체 정렬(8바이트)과 맞고 `bytemuck` 패딩이 안 생김 |
| S2 | 페이지 B+tree 용량 | `max_keys` 인자 유지(1단계 T6), 상한 = 페이지에 들어가는 수(리프 ≈ 1022) | 테스트는 4로 분할 강제, 실험은 페이지 최대 |
| S3 | Hash 디렉터리 페이지 | 메타 페이지에 디렉터리 페이지 번호 배열, 디렉터리 페이지당 칸 2048개, 지역 깊이는 버킷 페이지 헤더 | 두 배 = 디렉터리 페이지 복사·추가 쓰기로 카운터에 잡힘 |
| S4 | 파일 저장소 | 인덱스 하나 = 파일 하나, 페이지 0 = 메타 | 단순, 재열기 경계 명확 |
| S5 | 삽입 중 핀 | 조상은 쥐고 내려가지 않음 — 분할은 1단계처럼 `Split`을 돌려주고 부모를 다시 fetch | P6(핀 상한) |

## 11. 디테일 계획 — diff 단위 (각 diff = lazy 이해 게이트 1회)

| diff | 내용 | 의존 |
|---|---|---|
| 01 | `Cargo.toml`(bytemuck, dev tempfile) · `storage` 모듈: `PAGE_SIZE`·`PageId`·저장소 trait + 메모리 저장소 · Pager(read/write 카운터) (+ A1 스모크) | — |
| 02 | 파일 저장소(`read_at`/`write_at`) + 재열기 | 01 |
| 03 | 버퍼풀: 프레임·페이지 테이블·핀 가드(`RefCell`)·dirty·write-back·LRU·`flush`·통계 (+ A2 스모크 준비) | 01 |
| 04 | 버퍼풀 테스트(블라인드 워커) — P2 정확 수치 | 03 |
| 05 | 페이지 레이아웃(`bytemuck`): 메타·B+tree 리프·내부 페이지 + 용량 계산 + 왕복 테스트 | 01 |
| 06 | 페이지 B+tree: `get` + 리프 삽입·리프 분할(copy-up)·형제 링크 | 03, 05 |
| 07 | 페이지 B+tree: 내부 분할(push-up)·루트 분할·메타 갱신·`range`/`range_rev`·`impl Index`·재열기 (A2 실증) | 06 |
| 08 | 페이지 B+tree 테스트(블라인드) — 작은 풀 proptest·`check()`·핀 누수 0·재열기 | 07 |
| 09 | 페이지 Hash 레이아웃(디렉터리·버킷 페이지) + `get` | 03, 05 |
| 10 | 페이지 Hash `insert`·분할·디렉터리 두 배(페이지 쓰기) | 09 |
| 11 | 페이지 Hash `range`·`impl Index`·재열기 + 테스트(블라인드) | 10 |
| 12 | 변이 점검 3건 + clippy·fmt + 완료 요약 | 04, 08, 11 |

## 12. study-note 기록

- 1단계와 같은 `study-note/lab/storage-labs/index-lab/README.md` 끝에 2단계 기록 append(규칙: 기존 기록 수정 금지).
- 학습 문서: `docs/learning/stage2/`(1단계 `stage1/`과 같은 방식 — 설명 + 오답 노트).
- 학습 틈 선행 1문(1단계 NEXT N2): 2단계 첫 게이트 시작에 힌트 없이 "버킷에 key 대신 mix(key)를 저장하면 range와 check()에서 무엇이 깨지나?" + `check()`가 무엇을 검사하는지 본인 말로.

## 승인 상태

- [x] 필수 6칸 전부 기입
- [x] 트레이드오프 S1~S5 사용자 선택 — 제안대로 (2026-11-12)
- [x] 계획 이해 게이트 통과 (2026-11-12, 독립 판정 PASS 1/2 — 보완: 핀은 읽기 중인 페이지도 보호 · 작은 풀에서는 dirty 내보내기로 write도 증가 · B+tree 내부 노드/Hash 디렉터리 페이지가 각각 hot set · 범위 조회는 B+tree, 디렉터리 두 배는 Hash에 불리 → 3단계 가설 정교화)
- [x] 사용자 합의 → SPEC=1 (2026-11-12)
- [x] 자율성 선택 → lazy

# log — stage1-inmemory-index (lazy)

## 타임라인

| 시각 | 사건 | 결과/결정 |
|------|------|----------|
| 2026-11-04 | 리서치(내부·외부) + 트레이드오프 T1~T13 사용자 선택 | 명세 §9·§10 |
| 2026-11-08 | 계획 이해 게이트 — Q1(구조)·Q2(2단계 대비)·Q3(검증) 사용자 주관식 → 독립 워커(Opus) 판정 | 1/2회차 PASS. 보완 권고: 내부 노드도 구분자 키 보유, copy-up(리프에 남음) vs push-up(빠짐) |
| 2026-11-08 | 명세 승인(SPEC=1) · lazy · 브랜치 feat/stage1-inmemory-index | log 생성 → 게이트 재기록 |
| 2026-11-08 | diff 01a 게이트 1/2 — 판정 워커(Opus) | fail: get·range·range_rev 약속 누락, trait 구현 주체·정답지 비교, const가 무엇을 언제 막는지 누락 → 재질문 |
| 2026-11-08 | diff 01a 게이트 2/2 — 판정 워커(Opus) | fail → 지적 후 통과 처리: range_rev(같은 범위 내림차순) 누락, const = 컴파일 시점에 dyn 비호환 변경 감지(3단계 Box<dyn Index>) 설명 부족 |
| 2026-11-08 | diff 01b lib.rs 연결 → cargo test·clippy -D warnings·fmt --check 통과. 변이: trait에 제네릭 메서드 추가 → E0038 "not dyn compatible"로 컴파일 실패 확인. 복원 실수: src 미추적이라 git checkout이 실패해 변이가 남음 → sed로 그 줄 삭제·mod.rs가 01a 스니펫과 동일함 재확인 | 01b 게이트 1/2 pass (Opus) |
| 2026-11-08 | diff 01c Cargo.toml dev-dep proptest = "1.12.0" → A1 실증: rustc 1.92.0에서 proptest v1.12.0 컴파일, cargo test exit 0, clippy -D warnings 통과. 런타임 의존성 0(cargo tree -e normal), dev 의존성 proptest 1개 | 01c 게이트 1/2 pass (Opus) |
| 2026-11-08 | diff 01 커밋(690e579 코드 · dfc9207 docs) → diff 02a src/index/btree.rs(노드 구조체 + get). mod 선언은 02b(쓰기 1회=게이트 1회). 검증: rustc 단독 컴파일 -D warnings 통과 + scratch 스모크(손으로 만든 [30]→[10,20]|[30,40] 트리에서 get 8케이스·find_leaf(29)=왼쪽·(30)=오른쪽) 통과 | 02a 게이트 1/2 fail (Opus): 리프 binary_search·없으면 None 누락, "같으면 오른쪽"의 이유(구분자 규칙·구분자 출처) 누락 → 재질문 · 2/2 pass (Opus) |
| 2026-11-08 | diff 02b mod.rs에 `pub mod btree;` → cargo test exit 0 · clippy -D warnings 통과(expect 충족) · **fmt --check 실패**: btree.rs의 `#[expect(dead_code, reason = …)]` 한 줄이 길어 rustfmt가 여러 줄로 바꾸려 함 → btree.rs를 다시 쓰는 diff 03에서 함께 정리(Bash `cargo fmt`로 게이트 우회하지 않음). 커밋은 fmt 통과 후 | 02b 게이트 1/2 pass (Opus) |
| 2026-11-08 | diff 03 btree.rs insert·insert_at·split_leaf + 루트 분할(새 루트)을 04에서 03으로 당김(첫 리프 분할부터 필요) — 04에는 내부 분할만 남음. scratch 스모크 4/4(순차·역순 60개 max 3, 섞인 200개 max 4·2: insert 반환·get이 BTreeMap과 일치, next 체인 오름차순·prev 체인 내림차순 전체 일치, 첫 분할 [30]→[10,20]|[30,40]). **clippy 실패**: Node의 expect(dead_code) 미충족(split_leaf에서 prev를 바인딩해 읽음으로 집계) · **fmt 실패**: 주석 앞 공백 2칸 → 03b에서 수정 | 03 게이트 대기 |
| 2026-11-09 | diff 03 게이트 1차 답변 판정(Opus 워커) — 전파 경로(재귀 반환·keys/children 끼우기)만 충족, split_leaf 동작·구분자(오른쪽 첫 키 복사)·링크 재연결·루트 분할(새 루트) 누락 | fail 1/2 — 정답 비노출 재질문 |
| 2026-11-09 | diff 03 2차 시도 대신 사용자가 설명 요청 → 2차 기회 포기로 보고 fail 2/2 규칙 적용(정답 설명 후 통과 처리) — 누락 4점(반 나눔·오른쪽 첫 키 복사·링크 4개·새 루트) 설명 | 03 통과(2/2 설명) |
| 2026-11-09 | diff 03b btree.rs — Node의 `#[expect(dead_code)]` 삭제(prev 바인딩으로 미충족) · 주석 앞 공백 2→1(fmt). 사용자 요청: 커밋·push 후 다음 진행 → cargo test exit 0 · clippy -D warnings 통과 · fmt --check 통과 · scratch 스모크 4/4 재통과. 사용자 요청으로 구조·알고리즘 그림 문서 docs/learning/stage1/btree.md 작성 | 03b 커밋 8f5a085(코드)·8d8c884(docs), 브랜치 push(사용자 요청) |
| 2026-11-09 | diff 03b 게이트 — 판정 워커(Opus) | PASS 1/2 (보충: -D warnings가 unfulfilled_lint_expectations 경고를 오류로 올림 — 답변에서 생략, 허용) |
| 2026-11-09 | diff 04 btree.rs — 내부 노드 넘침 시 split_internal(push-up: 가운데 키를 위로 이동, 오른쪽 키·자식을 새 Internal로) · ponytail 주석 제거 — **스니펫만 기록, 코드 미적용**. 사용자 요청: 지금까지를 main에 ff 병합(8d8c884) → 새 브랜치 feat/stage1-internal-split, 세션 끊고 재개 | 같은 세션에서 이어 진행(사용자) |
| 2026-11-09 | diff 04 코드 적용(feat/stage1-internal-split, 기록한 스니펫 그대로) → cargo test exit 0 · clippy -D warnings · fmt --check 통과. scratch 스모크 6/6: 기존 4개 + 10~100(max 3) 루트 [70]·자식 [30,50]|[90]·깊이 3, 3000개×max 2·3·4·7에서 BTreeMap 일치·next/prev 체인·구조 검사(키 수 ≤ max, 내부 키≥1, children=keys+1, 리프 깊이 동일) | 04 게이트 대기 |
| 2026-11-09 | 사용자 요청: 커밋 21d393d(명세·로그) 날짜를 실제 작업일 11-07 23:00으로 변경 → commit-tree로 이후 체인 재작성(트리 동일 확인). 매핑 21d393d→db5e3d2 · 6d67b5f→690e579 · d1841c6→dfc9207 · e63d31c→8f5a085 · 664ec96→8d8c884 · 01fbcde→eddd948. 이 로그의 SHA 참조 일괄 갱신. main·feat/stage1-inmemory-index force-with-lease push | 04 게이트 대기 유지 |
| 2026-11-09 | diff 04 게이트 1차 — 판정 워커(Opus) | fail 1/2: 나누는 방식·Split 동일 형태로 인한 전파 누락, push-up은 "위로 등록"만(아래에 안 남음·copy-up 대비 누락). 오개념 2: "중간값이 더 빨리 도달"(get은 항상 리프까지), "값이 리프에 몰린 구조가 아니다"(B+tree는 값이 리프에만) → 정답 비노출 재질문 |
| 2026-11-09 | diff 04 게이트 2차 — 판정 워커(Opus) | fail 2/2 → 정답 설명 후 통과 처리: 자식 분배(mid+1개 왼쪽)·오른쪽은 새 노드, 가운데 키는 내부 노드에서 빠짐(리프 copy-up과 대비), Split 반환 형태가 같아 부모·루트 코드 무변경 전파 설명. 잔여 오개념(가운데 키가 리프에서 올라온다) 정정 |
| 2026-11-09 | 사용자 요청: diff 04 Q&A(키·자식 개수, 칸막이·구간, 70 귀속)와 diff 03~04 게이트 오답 노트를 docs/learning/stage1/btree-qa.md로 문서화 | 문서 커밋 |
| 2026-11-09 | 브랜치 feat/stage1-range(feat/stage1-internal-split 위에서 분기 — 주제별 브랜치 관례). diff 05 btree.rs — range(find_leaf(lo)부터 next) · range_rev(find_leaf(hi)부터 prev) · `impl Index for BTree`(4메서드 위임) | 05 코드 적용 |
| 2026-11-09 | diff 05 → cargo test exit 0 · clippy -D warnings · fmt --check 통과. scratch 스모크 3/3: max 2·3·4·7 × 키 1500개에서 경계점 13×13 조합 + 무작위 800구간(lo>hi 포함) + [0,MAX]·[MAX,MAX] 모두 BTreeMap::range와 일치(range·range_rev, dyn Index 경유), 빈 트리·키 1개, 그림 예 range(25,65)=[30,40,50,60] | 05 게이트 대기 |
| 2026-11-09 | diff 05 게이트 1차 — 판정 워커(Opus) | fail 1/2: 시작점(lo·hi)만 충족, next/prev 링크·정지 조건(k>hi / k<lo, hi 포함)·내부 노드 1회·C.prev 누락 시 빠지는 키(A')와 이유 누락 → 정답 비노출 재질문. 사이드 질문(역순 필요성)은 ORDER BY DESC LIMIT·T7/T13 근거로 답변, iterator화는 NEXT 후보 |
| 2026-11-10 | diff 05 게이트 2차 — 사용자 요청으로 ①~④를 하나씩 질의(코드만 제시, 정답 비노출) → 합쳐서 판정 워커(Opus) | PASS 2/2 (사소한 생략: 체인 끝 정지). 사이드 질문 "C.prev가 왜 꼭 필요한가" — 각 방향 체인에서 A'로 들어가는 길이 하나뿐(I2) 답변 |
| 2026-11-10 | 사용자 요청: main에 ff 병합(b8887da, diff 04·05 포함) → test·clippy·fmt 재확인 → main push → 브랜치 feat/stage1-btree-tests. diff 06 테스트 설계는 블라인드 워커(Opus — spec·trait·타입 정의만, 알고리즘 본문 미열람)에 위임 | 06 설계 회수 |
| 2026-11-10 | 사용자 요청: diff 05 설명(range/range_rev 동작·비교표, 내부 노드 1회 하강, 역순 필요성, C.prev 필요성, impl Index)을 btree.md §10으로 문서화 | 문서 커밋 |
| 2026-11-10 | diff 06 블라인드 설계 회수(Opus, 토큰 86.6k) — check()+단위 13+proptest 2, 워커 사본에서 15/15·clippy·fmt 통과, check() 자체를 손상 4종(prev 끊기·구분자 -1·고아·과소 리프)으로 검증. **명세 편차 발견**: 내 브리프가 check를 테스트 전용으로 지시 ↔ 명세 T12 = depth·check 공개 → T12대로 `pub fn depth`·`pub fn check`(+private leaf/Walk/walk)로 재배치, 테스트 로직 무변경. 스크래치 사본 15/15·clippy -D warnings·fmt·비테스트 빌드 경고 0 | 06 코드 적용 |
| 2026-11-10 | diff 06 적용(스크래치 검증본과 바이트 동일) → cargo test 15/15(≈1.1s) · clippy -D warnings · fmt --check 통과. 맛보기 변이(스크래치 사본, find_leaf `sep <= key`→`sep < key`): 4개 테스트 실패로 검출(range_boundaries·upsert_after_splits·dyn_index·proptest matches_btreemap) — 정식 변이 2건은 diff 10 | 06 게이트 대기 |
| 2026-11-10 | 사용자 요청: diff 06 설명(블라인드 설계·check 검사 항목·범위 물려받기·하한 출처·단위/proptest 표·변이 맛보기)을 btree.md §11로 문서화 | 문서 커밋 |
| 2026-11-10 | 06 게이트 1차 답변 "check는 위에서 lo hi 범위 물려받아서 확인하고 proptest는 BTreeMap이랑 비교" → Opus 판정 FAIL(①부분: 자식별 좁혀지는 경계 누락 ②부분: 비교 대상 누락 ③누락: 삽입마다 check 이유) | 1/2 실패 · 재질문 |
| 2026-11-11 | 06 게이트 2차(사용자 선택: 하나씩) — ①자식 범위 c0[P_lo,30)·c1[30,70)·c2[70,P_hi) 스스로 도달 ②비교 대상 일부만(insert 반환값·get·range_rev·dyn 경유는 정정 받음) ③"삽입 시 확인해 깨짐 방지"(시점만, 놓치는 상황·실패 위치 특정 누락, "방지"는 오개념 → 검출) → Opus 판정 FAIL | 2/2 실패 → 정답 설명 후 통과 |
| 2026-11-11 | gate-pass → cargo test 15/15 재확인 → 코드 커밋 bba4e0a | 06 완료 |
| 2026-11-11 | main ff 병합(0d9d87a) · push → 브랜치 feat/stage1-hash-index. diff 07a 설계: hash.rs(구조체·mix·new·get·slot) — 스크래치 검증 cargo test 15/15·clippy -D warnings·fmt 통과, 스모크(get None/Some · mix 0..100000 충돌 0, mix(0)=0 고정점) | 07a 적용 대기 |
| 2026-11-11 | 07a Write(스크래치 검증본과 동일) · 게이트 1차 답변 "칸으로 먼저 확인하고 버킷번호로 탐색 … 믹스를 통해 균일하게 … 깊이 2이면 2의 2배 4개 칸 … 끝자리가 1인지 보기에 … 버킷 3개" → Opus 판정 FAIL(①mix→하위 비트→칸 연결 누락 ②원리 맞음·일반식/‘2의 2배’ 판별 불가 ③‘균일’ 일반론, 8의 배수 실패·전단사 누락) | 1/2 실패 · 재질문 |
| 2026-11-11 | 07a 게이트 2차(하나씩) — "mix로 1을 빼서"(mix↔mask −1 혼동 → 정정) · mask 0b0111 · 8칸 직접 셈 · 8의 배수 → 디렉터리만 커지고 겹침 직접 추론 · "mix로 균등" → Opus 판정 PASS(근소 — 전단사·칸 공유 조건 미언급은 오답노트로) | 07a 통과 |
| 2026-11-11 | diff 07b 설계: mod.rs에 `pub mod hash;` 1줄 | 07b 적용 |
| 2026-11-11 | 07b 적용(첫 Edit은 미Read로 거부 → Read 후 재적용) → cargo test 15/15 · clippy -D warnings(expect 충족) · fmt 통과. 사용자 질문 "섞이면 디렉터리가 어떻게 생기나" → 8·16·24 실계산(키 그대로 d=4까지 16칸 vs mix d=1 2칸) 답변. 게이트 1차 "그 한 줄은 로컬 라이브러리였을 것" → Opus 판정 FAIL(모듈 트리 미연결 → 미컴파일 인과 없음) | 1/2 실패 · 재질문 |
| 2026-11-11 | 사용자 요청: 게이트 답을 판정 전에 메인이 먼저 확인·힌트로 고쳐 가기(사전 점검 라운드는 x/2 미소모) — 메모리 저장. 07b 2차(사전 점검 거침): "pub 없으면 컴파일 안 됨, 임포트가 없어서"(pub 혼동 → 정정) · "외부에서 못 읽어서 없다고 나옴"(pub=공개) · "컴파일이 애초에 안 돼서" → Opus 판정 PASS(근소 — Q1 파일 목록 본인 미기재) → gate-pass · 코드 커밋 3f80cc1 | 07 완료 |
| 2026-11-11 | 문서 커밋 4883d5c(hash.md). diff 08 설계: insert(upsert → push → 넘치면 split 반복) · split(지역=전역이면 디렉터리 extend_from_within 두 배 → d번째 비트로 재배치 → 디렉터리 칸 재지정, 전체 훑기 ponytail) · dead_code expect 2개 제거. 스크래치: 첫 시도 E0502(self.slot 빌림) → id 먼저 계산으로 수정. 스모크(release): cap 1..4 × seed 30 × 300 삽입 — insert 반환·get이 BTreeMap과 일치, 삽입마다 I3 검사 통과, 최대 전역 깊이 15(cap 1). 8·16·24 cap2 → 전역 1 dir [0,1](hash.md §3 계산과 일치), 16·32·40 → 전역 4. 첫 스모크는 debug + cap1 + 매 삽입 O(디렉터리×버킷) 검사로 120s 초과 → 중단·경량화. cargo test 15/15·clippy -D warnings·fmt 통과 | 08 적용 |
| 2026-11-11 | 08 Write(스크래치 검증본과 동일) · test 15/15·clippy·fmt. 게이트 1차(사전 점검 거침, 하나씩): Q1 "B는 디렉터리 2개로 늘고… A는 내부 확장/밀림"(①② 디렉터리 뒤바뀜 → 정정) → "4→8, next·prev에 끼우는 형식"(B+tree 혼동 → 정정) · Q2 "3번째 비트만 다르고 B는 01만 보니 같은 칸" · Q3 "32,40이 몰려 다시 넘치니 반복"(근거 사례 부정확) → Opus 판정 FAIL(Q1 약함·Q2 일반화 누락·Q3 재조회 누락) | 1/2 실패 · 재질문 |
| 2026-11-11 | 사용자 요청: 08 설명 문서화 → hash.md §4-1(insert·split 코드·두 경우·복사 이유·next/prev 없음·16/32/40 loop 따라가기 표, 실행 출력과 대조) + §6 08 1차 오답 노트 · 사용자 질문("디렉터리 링크 때문에 문제?") 답 기록 | 문서 |
| 2026-11-11 | 08 게이트 2차(사전 점검·하나씩, 재질문 P/Q/R·X 예시): (a) 첫 답 "복사되며 확장" ✗ → 코드 따라가기로 d=1·조건 거짓·확장 없음 ✅, bit=0b1 ✗(0b10), 11→R' ✅ · (b) 첫 답 전부 ✗(거짓·4칸·R'·재해싱·2^n 이동) → 참·8칸·복사 매핑·000/100→P ✅, "지역 깊이 3이 된다" ✗, 이동 0개 ✅(힌트 뒤) · (c) 셋 다 X'·0 남고 1 이동 일반화·key로 X' 재조회 ✅ → Opus 판정 FAIL(c만 독립 충족) | 2/2 실패 → 정답 설명 후 통과 |
| 2026-11-11 | gate-pass → test 15/15 → 코드 커밋 98a6e82 | 08 완료 |
| 2026-11-11 | 문서 커밋 f80fc92. diff 09를 09a(range·range_rev·impl Index — 메인 구현)와 09b(global_depth·bucket_count·check 공개 + 테스트 — 블라인드 워커, B+tree 05/06과 같은 분리)로 나눔. 09a 스크래치: 전체 버킷 훑기→필터→sort_unstable, rev=뒤집기; 스모크(release) cap 1..4 × seed 30 × 200 삽입 × 범위 6종(lo>hi·한 점·u64::MAX 포함) BTreeMap·dyn Index 일치; test 15/15·clippy·fmt 통과 | 09a 적용 |
| 2026-11-11 | 09a 게이트(사전 점검·하나씩): Q1 "옆 칸에 없고 연결도 없으니 전부 훑음" ✅ · Q2 첫 답 "난수값이라 역정규화 후 정렬" ✗(원래 키 저장) → "원래 키로 정렬" · "버킷은 분할 때 번호, 키는 넣은 순서라 오름차순 아님" ✅ · Q3 "이미 정렬 → 뒤집으면 내림차순, 어차피 전체 훑기" ✅ → Opus 판정 PASS(남은 틈: trait 오름차순 요구 명시 X, mix/원키 혼동 재확인) → gate-pass → test 15/15·clippy·fmt → 코드 커밋 d40fe20 · hash.md §4-2·§5·§6 09a 갱신 | 09a 완료 |
| 2026-11-11 | 문서 커밋 cc418da. diff 09b 테스트 설계를 블라인드 워커(Opus)에 위임 — 스크래치 사본(git archive HEAD cc418da)에서 작업, 열람 허용 = spec·mod.rs·hash.rs 타입/mix/new/get/slot/impl Index + insert·range 시그니처만(insert·split·range 본문 미열람). 지시: T12 공개 API(global_depth·bucket_count·check — I3 전체) + 단위(두 배 두 경우 강제·반복 분할·범위 경계·dyn) + proptest(cap 2..=4, BTreeMap·dyn 대조, 매 삽입 check) + check 손상 검증 ≥3종 | 09b 위임 |
| 2026-11-11 | 09b 블라인드 설계 회수(Opus, 토큰 78.6k) — 열람 누수 자진 보고: insert 본문 첫 줄(58)·split 닫는 괄호(120~121)·range 첫 줄(126), 테스트 설계엔 무영향. 산출: pub global_depth·bucket_count·check(I3 전체 + 고아 버킷 위반 — 근거 T4 삭제 없음) + 단위 11 + check 손상 검증 10(should_panic expected 문자열로 해당 assert 지목) + proptest 2(matches_btreemap cap 2..=4·300 ops·dyn 대조 / structure_holds_after_every_insert 매 삽입 check·깊이·버킷 수 단조·upsert 무변화). 메인 검토: 원본 166줄 무변경(추가만) 확인, 두 배 시나리오 키 선택을 손으로 추적해 기대값(전역 1→2 버킷 2→3→4, 반복 분할 전역 4 버킷 5) 맞음 확인. 레포 적용 → test 38/38(1.8s)·clippy -D warnings·fmt·비테스트 빌드 경고 0 | 09b 적용 |
| 2026-11-12 | 09b 게이트(사전 점검·하나씩): Q1(a) 4칸·접미사 1 칸 나열 ✅ · Q1(b) "mix키" ✗(09a 혼동 재발) → "그냥 키" ✅ → "그냥 키는 서브픽스" ✗ → 키 30 예시 후 "칸은 mix로 정해지니 mix로 맞춰야" ✅ · Q2 "1개라 통과, mix 000 ≠ 접미사 100이라 터짐" ✅ · Q3 "깨지는 지점 바로 잡음, cap1 깊어짐" → "32768칸이라 느림, 100이면 분할 거의 없음" ✅ → Opus 판정 PASS(경계선 — 원키/mix 구분이 두 게이트 연속 첫 물음에 실패, 다음 게이트에서 힌트 없이 1문 선행) → gate-pass → test 38/38 → 코드 커밋 9d9f99e · hash.md §4-3·§5·§6 09b 갱신 | 09b 완료 |
| 2026-11-12 | 문서 커밋 2c5605b. diff 10 변이 점검(스크래치 mut10, HEAD 2c5605b 사본 — 레포 무변경, 각 변이 후 원복·baseline 38/38 재확인): B1 split_leaf `sep = right_keys[0]` → 왼쪽 마지막 키 → 10 실패(first_leaf_split_copies_up_separator·internal_split…·matches_btreemap·structure_holds_after_every_insert·range_boundaries 등) · B2 find_leaf `sep <= key` → `sep < key` → 4 실패(matches_btreemap·range_boundaries·upsert_after_splits_keeps_structure·works_through_dyn_index) · H1 칸 재지정 `bit != 0` → `== 0`(반대 절반) → 15 실패(두 배 단위 3종·proptest 2·range_edges·dyn, sample() 기반 check 손상 테스트 7종은 sample()의 check가 먼저 터져 should_panic expected 불일치로 실패) · H2 칸 재지정 생략 → 단위 6 + proptest 2 실패(hash.rs:226 칸 수 검사, matches_btreemap 축소에 ~110s) — 4종 모두 검출 | 10 변이 |
| 2026-11-12 | 10 게이트(사전 점검): Q0 힌트 없이 "mix로 바이트화한 값 저장·그대로 정렬" ✗(3연속) → 스크래치에서 실제 버킷 내용 출력(30~33) 보여 줌 → "원래 숫자, mix는 칸 정할 때만"·"range는 원래 키" ✅, 사용자 "check는 무슨 역할?" → 역할 설명(사서 비유·표) · Q1 B2 "키=구분자면 좌측, get None으로 실패" ✅ · Q2 H2 "옮겨진 키 못 찾음, 1칸이어야 하는데 A 2칸 A′ 0칸" ✅ → Opus 판정 PASS(변이 이해 한정; 원키/mix 구분은 여전히 틈 → 2단계 첫 게이트 선행 1문) → gate-pass · 코드 변경 없음 · hash.md §4-4·§5·§6 10 갱신 | 10 완료 |

## diff 기록 (before/after — 판정 워커 그라운드 트루스)

### diff 01a — `cargo init --lib` + `Index` trait

- T14 추가(diff 직전 사용자 선택): `range` 경계 = 양끝 포함 `[lo, hi]`.
- 훅이 파일 쓰기 1회마다 게이트를 걸어 diff 01을 01a(trait) / 01b(Cargo.toml dev-dep·lib.rs 연결)로 나눔.

`cargo init --lib --name index-lab --vcs none` 생성물 (before: 없음):

```toml
# Cargo.toml
[package]
name = "index-lab"
version = "0.1.0"
edition = "2024"

[dependencies]
```

`src/index/mod.rs` (before: 없음 → after):

```rust
pub trait Index {
    /// `key`에 `value`를 넣는다. 같은 키가 이미 있으면 값을 바꾸고 이전 값을 돌려준다(upsert).
    fn insert(&mut self, key: u64, value: u64) -> Option<u64>;

    /// `key`의 값을 돌려준다. 없으면 `None`.
    fn get(&self, key: u64) -> Option<u64>;

    /// `lo <= key <= hi`인 (키, 값)을 키 오름차순으로 돌려준다. `lo > hi`면 빈 결과.
    fn range(&self, lo: u64, hi: u64) -> Vec<(u64, u64)>;

    /// `range`와 같은 범위를 키 내림차순으로 돌려준다.
    fn range_rev(&self, lo: u64, hi: u64) -> Vec<(u64, u64)>;
}

// 3단계 러너가 `Box<dyn Index>`로 구현을 바꿔 끼운다 — trait가 dyn으로 못 쓰이게 바뀌면 여기서 컴파일이 깨진다.
const _: Option<&dyn Index> = None;
```

### diff 01b — `src/lib.rs` 모듈 연결

before (`cargo init` 템플릿):

```rust
pub fn add(left: u64, right: u64) -> u64 {
    left + right
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn it_works() {
        let result = add(2, 2);
        assert_eq!(result, 4);
    }
}
```

after:

```rust
//! index-lab — 페이지 기반 스토리지 엔진 위 인덱스 실험. 1단계: 메모리 내 B+tree · Hash.

pub mod index;
```

### diff 01c — `Cargo.toml` dev-dependency

before (`cargo init`):

```toml
[package]
name = "index-lab"
version = "0.1.0"
edition = "2024"

[dependencies]
```

after:

```toml
[package]
name = "index-lab"
version = "0.1.0"
edition = "2024"

[dev-dependencies]
proptest = "1.12.0"
```

### diff 02a — `src/index/btree.rs` 노드 구조체 + `get` (before: 없음)

```rust
//! B+tree — 노드를 배열(`Vec<Node>`)에 두고 배열 번호(`NodeId`)로 연결한다(T5).
//! 2단계에서 `NodeId`가 그대로 페이지 번호(`PageId`)가 된다.

/// 노드의 배열 번호.
pub type NodeId = usize;

#[expect(dead_code, reason = "Internal 생성·prev/next·max_keys는 diff 03~05에서 사용")]
enum Node {
    /// 이정표 노드: 키 n개 + 자식 n+1개. 구분자 규칙 = 왼쪽 < 구분자 ≤ 오른쪽.
    Internal {
        keys: Vec<u64>,
        children: Vec<NodeId>,
    },
    /// 데이터 노드: `keys[i]`의 값 = `values[i]`. 옆 리프와 양방향 연결(T7).
    Leaf {
        keys: Vec<u64>,
        values: Vec<u64>,
        prev: Option<NodeId>,
        next: Option<NodeId>,
    },
}

pub struct BTree {
    nodes: Vec<Node>,
    root: NodeId,
    #[expect(dead_code, reason = "분할 기준 — diff 03에서 사용")]
    max_keys: usize,
}

impl BTree {
    /// `max_keys` = 노드 하나에 들어가는 최대 키 수(fan-out, T6).
    /// 1이면 내부 분할 때 오른쪽 노드가 키 없이 남으므로 2 이상만 받는다.
    pub fn new(max_keys: usize) -> Self {
        assert!(max_keys >= 2, "max_keys는 2 이상이어야 한다");
        let empty_leaf = Node::Leaf {
            keys: Vec::new(),
            values: Vec::new(),
            prev: None,
            next: None,
        };
        BTree {
            nodes: vec![empty_leaf],
            root: 0,
            max_keys,
        }
    }

    pub fn get(&self, key: u64) -> Option<u64> {
        let Node::Leaf { keys, values, .. } = &self.nodes[self.find_leaf(key)] else {
            unreachable!("find_leaf는 리프만 돌려준다");
        };
        keys.binary_search(&key).ok().map(|i| values[i])
    }

    /// 루트에서 구분자를 따라 내려가 `key`가 있어야 할 리프의 번호를 돌려준다.
    fn find_leaf(&self, key: u64) -> NodeId {
        let mut id = self.root;
        loop {
            match &self.nodes[id] {
                Node::Internal { keys, children } => {
                    // key 이하인 구분자 개수 = 내려갈 자식 칸 (key == 구분자면 오른쪽으로)
                    let i = keys.partition_point(|&sep| sep <= key);
                    id = children[i];
                }
                Node::Leaf { .. } => return id,
            }
        }
    }
}
```

### diff 02b — `src/index/mod.rs` 모듈 선언

before:

```rust
//! 키는 `u64`, 값은 `u64` 고정(README 범위: 키 `u64`, 값 고정 길이).

/// 키 → 값 인덱스.
```

after:

```rust
//! 키는 `u64`, 값은 `u64` 고정(README 범위: 키 `u64`, 값 고정 길이).

pub mod btree;

/// 키 → 값 인덱스.
```

### diff 03 — B+tree `insert` · 리프 분할(copy-up) · 형제 재연결 · 루트 분할

before = diff 02a 스니펫. 변경분(`diff -u`, 실파일):

```diff
@@ -4,7 +4,10 @@
 /// 노드의 배열 번호.
 pub type NodeId = usize;
 
-#[expect(dead_code, reason = "Internal 생성·prev/next·max_keys는 diff 03~05에서 사용")]
+/// 노드가 갈라졌을 때 부모에게 알리는 것: (부모에 넣을 구분자, 새로 생긴 오른쪽 노드).
+type Split = (u64, NodeId);
+
+#[expect(dead_code, reason = "Leaf.prev는 diff 05 range_rev에서 읽는다")]
 enum Node {
     /// 이정표 노드: 키 n개 + 자식 n+1개. 구분자 규칙 = 왼쪽 < 구분자 ≤ 오른쪽.
     Internal {
@@ -23,7 +26,6 @@
 pub struct BTree {
     nodes: Vec<Node>,
     root: NodeId,
-    #[expect(dead_code, reason = "분할 기준 — diff 03에서 사용")]
     max_keys: usize,
 }
 
@@ -52,6 +54,86 @@
         keys.binary_search(&key).ok().map(|i| values[i])
     }
 
+    /// 같은 키가 있으면 값을 바꾸고 이전 값을 돌려준다(upsert).
+    pub fn insert(&mut self, key: u64, value: u64) -> Option<u64> {
+        let (old, split) = self.insert_at(self.root, key, value);
+        if let Some((sep, right)) = split {
+            // 루트가 갈라졌다 → 새 루트를 맨 위에 올린다. 깊이는 여기서만 1 늘어난다.
+            let new_root = self.nodes.len();
+            self.nodes.push(Node::Internal {
+                keys: vec![sep],
+                children: vec![self.root, right],
+            });
+            self.root = new_root;
+        }
+        old
+    }
+
+    /// `id` 아래에 넣는다. 반환 = (이전 값, `id`가 갈라졌으면 그 `Split`).
+    fn insert_at(&mut self, id: NodeId, key: u64, value: u64) -> (Option<u64>, Option<Split>) {
+        match &mut self.nodes[id] {
+            Node::Leaf { keys, values, .. } => {
+                match keys.binary_search(&key) {
+                    Ok(i) => return (Some(std::mem::replace(&mut values[i], value)), None),
+                    Err(i) => {
+                        keys.insert(i, key);
+                        values.insert(i, value);
+                    }
+                }
+                if keys.len() <= self.max_keys {
+                    return (None, None);
+                }
+                (None, Some(self.split_leaf(id)))
+            }
+            Node::Internal { keys, children } => {
+                let i = keys.partition_point(|&sep| sep <= key);
+                let child = children[i];
+                let (old, split) = self.insert_at(child, key, value);
+                if let Some((sep, right)) = split {
+                    // 갈라진 자식 바로 오른쪽 칸에 구분자와 새 자식을 끼운다.
+                    let Node::Internal { keys, children } = &mut self.nodes[id] else {
+                        unreachable!("id는 위에서 Internal이었다");
+                    };
+                    keys.insert(i, sep);
+                    children.insert(i + 1, right);
+                    // ponytail: 내부 노드 넘침(keys.len() > max_keys)은 아직 그대로 둔다 — diff 04에서 push-up 분할.
+                }
+                (old, None)
+            }
+        }
+    }
+
+    /// 넘친 리프 A를 반으로 나눠 오른쪽 절반을 새 리프 A'로 옮긴다.
+    /// 구분자 = A'의 첫 키를 복사(copy-up — 리프에도 남는다).
+    /// 연결: A ⇄ C  →  A ⇄ A' ⇄ C (C의 prev까지 고친다).
+    fn split_leaf(&mut self, id: NodeId) -> Split {
+        let new_id = self.nodes.len();
+        let Node::Leaf {
+            keys, values, next, ..
+        } = &mut self.nodes[id]
+        else {
+            unreachable!("split_leaf는 리프에만 쓴다");
+        };
+        let mid = keys.len() / 2;
+        let right_keys = keys.split_off(mid);
+        let right_values = values.split_off(mid);
+        let sep = right_keys[0];
+        let old_next = next.replace(new_id); // A.next = A'
+        self.nodes.push(Node::Leaf {
+            keys: right_keys,
+            values: right_values,
+            prev: Some(id),  // A'.prev = A
+            next: old_next, // A'.next = C
+        });
+        if let Some(c) = old_next {
+            let Node::Leaf { prev, .. } = &mut self.nodes[c] else {
+                unreachable!("리프의 next는 리프다");
+            };
+            *prev = Some(new_id); // C.prev = A'
+        }
+        (sep, new_id)
+    }
+
     /// 루트에서 구분자를 따라 내려가 `key`가 있어야 할 리프의 번호를 돌려준다.
     fn find_leaf(&self, key: u64) -> NodeId {
         let mut id = self.root;
```

### diff 03b — clippy(미충족 expect)·fmt 수정

```diff
-#[expect(dead_code, reason = "Leaf.prev는 diff 05 range_rev에서 읽는다")]
 enum Node {
```

```diff
-            prev: Some(id),  // A'.prev = A
+            prev: Some(id), // A'.prev = A
```

### diff 04 — 내부 노드 분할(push-up)

`insert_at`의 Internal 팔 — before:

```rust
                    keys.insert(i, sep);
                    children.insert(i + 1, right);
                    // ponytail: 내부 노드 넘침(keys.len() > max_keys)은 아직 그대로 둔다 — diff 04에서 push-up 분할.
                }
                (old, None)
```

after:

```rust
                    keys.insert(i, sep);
                    children.insert(i + 1, right);
                    if keys.len() > self.max_keys {
                        return (old, Some(self.split_internal(id)));
                    }
                }
                (old, None)
```

새 함수(before: 없음):

```rust
    /// 넘친 내부 노드를 가운데 키 기준으로 나눈다. 가운데 키는 위로 "이동"한다(push-up — 아래에 남지 않는다).
    /// keys [k0 .. k(mid-1)] · k(mid) · [k(mid+1) ..] → 왼쪽(id) · 부모로 · 오른쪽(새 노드)
    /// 자식은 mid+1개가 왼쪽에 남고 나머지가 오른쪽으로 간다.
    fn split_internal(&mut self, id: NodeId) -> Split {
        let new_id = self.nodes.len();
        let Node::Internal { keys, children } = &mut self.nodes[id] else {
            unreachable!("split_internal은 내부 노드에만 쓴다");
        };
        let mid = keys.len() / 2;
        let right_keys = keys.split_off(mid + 1);
        let sep = keys.pop().expect("mid < len이므로 가운데 키가 있다");
        let right_children = children.split_off(mid + 1);
        self.nodes.push(Node::Internal {
            keys: right_keys,
            children: right_children,
        });
        (sep, new_id)
    }
```

### diff 05 — `range` · `range_rev` · `impl Index for BTree`

before: `find_leaf`가 `impl BTree`의 마지막 메서드, 파일 끝.

after — `find_leaf` 뒤(`impl BTree` 안)에 추가:

```rust
    /// `lo <= key <= hi`를 오름차순으로. lo가 있어야 할 리프에서 시작해 next 링크만 따라간다(내부 노드 재방문 없음).
    pub fn range(&self, lo: u64, hi: u64) -> Vec<(u64, u64)> {
        let mut out = Vec::new();
        if lo > hi {
            return out;
        }
        let mut cur = Some(self.find_leaf(lo));
        while let Some(id) = cur {
            let Node::Leaf {
                keys, values, next, ..
            } = &self.nodes[id]
            else {
                unreachable!("리프의 next는 리프다");
            };
            let start = keys.partition_point(|&k| k < lo); // 첫 리프에서만 의미 있음(이후 리프는 전부 ≥ lo)
            for (&k, &v) in keys[start..].iter().zip(&values[start..]) {
                if k > hi {
                    return out;
                }
                out.push((k, v));
            }
            cur = *next;
        }
        out
    }

    /// `range`와 같은 범위를 내림차순으로. hi가 있어야 할 리프에서 시작해 prev 링크만 따라간다.
    pub fn range_rev(&self, lo: u64, hi: u64) -> Vec<(u64, u64)> {
        let mut out = Vec::new();
        if lo > hi {
            return out;
        }
        let mut cur = Some(self.find_leaf(hi));
        while let Some(id) = cur {
            let Node::Leaf {
                keys, values, prev, ..
            } = &self.nodes[id]
            else {
                unreachable!("리프의 prev는 리프다");
            };
            let end = keys.partition_point(|&k| k <= hi); // 첫 리프에서만 의미 있음(이전 리프는 전부 ≤ hi)
            for (&k, &v) in keys[..end].iter().zip(&values[..end]).rev() {
                if k < lo {
                    return out;
                }
                out.push((k, v));
            }
            cur = *prev;
        }
        out
    }
```

after — 파일 끝에 추가:

```rust
impl super::Index for BTree {
    fn insert(&mut self, key: u64, value: u64) -> Option<u64> {
        BTree::insert(self, key, value)
    }

    fn get(&self, key: u64) -> Option<u64> {
        BTree::get(self, key)
    }

    fn range(&self, lo: u64, hi: u64) -> Vec<(u64, u64)> {
        BTree::range(self, lo, hi)
    }

    fn range_rev(&self, lo: u64, hi: u64) -> Vec<(u64, u64)> {
        BTree::range_rev(self, lo, hi)
    }
}
```

### diff 06 — B+tree 테스트 + T12 구조 관찰 API

before: `btree.rs`는 `impl super::Index for BTree`로 끝남(테스트 없음, `depth`·`check` 없음).

after — 파일 끝에 추가(전문은 `src/index/btree.rs` 243행~ — 아래는 골격):

```rust
// ---------- 구조 관찰·검사 (T12: depth·check 공개) ----------
impl BTree {
    pub fn depth(&self) -> usize { /* 가장 왼쪽 경로로 내려가며 센 노드 수 */ }
    pub fn check(&self) {
        // walk(): 노드 1회 방문(공유·순환·고아 금지) · 키 ≤ max · 엄격 오름차순
        //         · 구분자 범위 [lo, hi) · 리프 values=keys · 비루트 하한(리프 ⌈m/2⌉, 내부 ⌊m/2⌋)
        //         · children = keys + 1 · 구분자 = 오른쪽 서브트리 최소 키
        // 모든 리프 깊이 같음 · 첫 리프 prev None · 끝 리프 next None
        // a.next = b ⇔ b.prev = a · next 사슬 = 중위 리프 순서(키 엄격 오름차순) · prev 사슬 = 역순
    }
}
fn leaf(..) / struct Walk / fn walk(..)        // check 보조(private)

#[cfg(test)]
mod tests {
    // 단위 13: new_tree_is_empty · new_with_max_keys_1_panics · min_max_keys_2_works
    //   · upsert_returns_old_value_and_replaces · upsert_into_full_leaf_does_not_split
    //   · upsert_after_splits_keeps_structure · no_split_until_overflow
    //   · first_leaf_split_copies_up_separator · internal_split_pushes_up_middle_and_grows_root
    //   · sequential_and_reverse_insertion · range_boundaries · keys_at_u64_extremes · works_through_dyn_index
    // proptest 2 (256 cases): matches_btreemap(max 2..=8, ops ≤400, 질의 ≤32 — get·range·range_rev·dyn Index vs BTreeMap)
    //   · structure_holds_after_every_insert(max 2..=5, ops ≤120 — 삽입마다 check)
    // 키 전략: 0..200 (6) · u64::MAX 근처 (1) · 전체 u64 (1)
}
```

### diff 07a — `src/index/hash.rs` 디렉터리·버킷 구조체 + 해시 함수 + `get` (before: 없음)

mod 선언(`pub mod hash;`)은 07b(쓰기 1회 = 게이트 1회, 02a/02b와 같은 방식).

```rust
pub type BucketId = usize;

struct Bucket {
    #[expect(dead_code, reason = "diff 08 분할에서 읽는다")]
    local_depth: u32,          // 이 버킷이 구분에 쓰는 하위 비트 수(≤ 전역 깊이, I3)
    keys: Vec<u64>,            // 순서 없음
    values: Vec<u64>,
}

pub struct Hash {
    directory: Vec<BucketId>,  // 길이 = 2^global_depth
    buckets: Vec<Bucket>,
    global_depth: u32,
    #[expect(dead_code, reason = "diff 08 넘침 판정에서 읽는다")]
    bucket_cap: usize,
}

fn mix(key: u64) -> u64 {     // splitmix64 마무리 — 전단사(T8)
    let mut z = key;
    z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    z ^ (z >> 31)
}

impl Hash {
    pub fn new(bucket_cap: usize) -> Self   // 전역 깊이 0 · 디렉터리 [0] · 빈 버킷 1개
    pub fn get(&self, key: u64) -> Option<u64> {
        let b = &self.buckets[self.directory[self.slot(key)]];
        b.keys.iter().position(|&k| k == key).map(|i| b.values[i])
    }
    fn slot(&self, key: u64) -> usize {      // 해시 하위 global_depth 비트
        let mask = (1u64 << self.global_depth) - 1;
        (mix(key) & mask) as usize
    }
}
```

### diff 07b — `src/index/mod.rs` 모듈 선언

```diff
 pub mod btree;
+pub mod hash;
```

### diff 08 — Hash `insert` · `split` (지역 < 전역 분할 · 지역 = 전역 디렉터리 두 배)

before: `Bucket.local_depth`·`Hash.bucket_cap`에 `#[expect(dead_code, …)]`, `insert` 없음.

after:
```rust
pub fn insert(&mut self, key: u64, value: u64) -> Option<u64> {
    let id = self.directory[self.slot(key)];
    let b = &mut self.buckets[id];
    if let Some(i) = b.keys.iter().position(|&k| k == key) {
        return Some(std::mem::replace(&mut b.values[i], value));
    }
    b.keys.push(key);
    b.values.push(value);
    loop {
        let id = self.directory[self.slot(key)];
        if self.buckets[id].keys.len() <= self.bucket_cap {
            return None;
        }
        self.split(id);
    }
}

fn split(&mut self, id: BucketId) {
    let d = self.buckets[id].local_depth;
    if d == self.global_depth {
        self.directory.extend_from_within(..);   // 두 배: 뒤 절반 = 앞 절반 복사
        self.global_depth += 1;
    }
    let bit = 1u64 << d;
    // 키를 꺼내 새 비트 0 → stay, 1 → moved (둘 다 local_depth d+1)
    // buckets[id] = stay · buckets.push(moved)
    for (slot, b) in self.directory.iter_mut().enumerate() {
        if *b == id && slot as u64 & bit != 0 {
            *b = new_id;                           // 새 비트가 1인 칸 → 새 버킷
        }
    }
}
```

### diff 09a — Hash `range`·`range_rev` · `impl Index for Hash`

before: `slot`으로 `impl Hash` 끝, trait 구현 없음.

after (`slot` 앞에 추가 + 파일 끝에 trait 구현):
```rust
pub fn range(&self, lo: u64, hi: u64) -> Vec<(u64, u64)> {
    let mut out: Vec<(u64, u64)> = self
        .buckets
        .iter()
        .flat_map(|b| b.keys.iter().copied().zip(b.values.iter().copied()))
        .filter(|&(k, _)| lo <= k && k <= hi)
        .collect();
    out.sort_unstable_by_key(|&(k, _)| k); // 키는 중복이 없으니 불안정 정렬로 충분
    out
}

pub fn range_rev(&self, lo: u64, hi: u64) -> Vec<(u64, u64)> {
    let mut out = self.range(lo, hi);
    out.reverse();
    out
}

impl super::Index for Hash { /* insert·get·range·range_rev → Hash::… 위임 (BTree와 같은 형태) */ }
```

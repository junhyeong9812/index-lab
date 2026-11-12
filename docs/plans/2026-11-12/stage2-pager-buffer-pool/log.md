# 작업 로그 — 로드맵 2단계: Pager + 버퍼풀

## 타임라인

| 시각 | 사건 | 결과 |
|---|---|---|
| 2026-11-12 | 사용자 "2단계 진행하자". 브랜치 feat/stage2-pager-buffer-pool(main 8cf93b9). 인터뷰 3회(11문 — 기존 구현·페이지 접근·디렉터리 위치·테스트 저장소·쓰기 정책·오류·영속성·바이트 변환·가드 동시성·카운터·순서), 모두 권장안 선택 | 인터뷰 완료 |
| 2026-11-12 | 리서치: cargo search/info(bytemuck 1.25.2·tempfile 3.27.0·rustc 1.92.0) · 웹(BusTub 버퍼풀·page guard·15-445 디스크 Extendible Hash 3단 페이지) → requirement-spec.md 작성(P1~P6·A1~A2·S1~S5·diff 01~12) | 명세 초안 |
| 2026-11-12 | S1~S5 제안대로 확정. 계획 이해 게이트(하나씩): Q1 "버퍼풀=페이지 캐시, 핀=쓰는 중 내보내기 막음, dirty=변경 유실 막음" · Q2 "PageId, pool.fetch 가드, drop 시 언핀" → (보충 질문) "id×16KB 위치, 고쳤으면 dirty 남음" · Q3 "miss 증가→read 증가, Hash가 랜덤 접근이라 타격 클 듯" → Opus 판정 PASS(보완: 읽기 핀·write 증가·hot set·범위/두 배 차이) | 계획 게이트 통과 |
| 2026-11-12 | 사용자 합의 + lazy 선택 → set-state spec-approved · mode lazy | L1 진입 |
| 2026-11-12 | NEXT N2 재확인(힌트 없이 1문: "버킷에 mix(key)를 저장하면 range·check에서 무엇이 깨지나"). check: "mix 다시 안 해도 됨" **힌트 없이 정답**(1단계 3회 연속 오답 지점) — 단 "검사 자체가 필요 없어진다"는 과잉, 위치 비교(저장값 하위 d비트 = 칸 접미사)는 남음. range: "mix 순서로 정렬" → (힌트) 필터 `lo<=k<=hi`가 mix 값과 원래 키를 비교 → 첫 답 "문자열 비교"(오개념: mix는 u64→u64 숫자) → "참이 안 됨"까지 도달, 결과(빈 Vec·반환 키가 mix 값)는 설명으로 보충 | 틈 대부분 해소 — 남은 틈: 필터가 정렬보다 먼저 깨진다는 연쇄 추론 |
| 2026-11-12 | diff 01 (스크래치 → 적용): `Cargo.toml` bytemuck(derive) + dev tempfile · `src/storage/mod.rs`(`PAGE_SIZE`·`PageId`·`Page`(align 8 Pod)·`Storage` trait·`MemStorage`) · `src/storage/pager.rs`(`Pager`: allocate=I/O 없음, 성공한 read/write만 계수) · `lib.rs`에 `pub mod storage`. **A1 스모크 첫 빌드 실패**: `[u8; 16384]: Zeroable` 미구현(E0277 — bytemuck 기본 배열 impl은 고정 길이 목록뿐) → feature `min_const_generics` 추가로 해소. 가드 변이 2종(헤더 u32 → "derive(Pod) was applied to a type with padding" / 배열 1022 → size assert) 모두 컴파일 오류로 검출 | test 41/41(38+3) · clippy·fmt 통과 · A1 실증 |
| 2026-11-12 | diff 01 게이트(하나씩): Q1 "실패해도 카운트가 올라가 실제 I/O보다 많이 셈" · Q2 "u64 8바이트 정렬이 안 맞아 캐스팅이 패닉" → 보충(실측 프로브: align(8) 없으면 오프셋 1에서만 `TargetAlignmentGreaterAndInputNotAligned`, 0·8은 성공 — 주소 운에 따라 간헐 실패) → Opus 판정 PASS(Q2 경계선: "항상 패닉"이 아니라 주소 의존) | 1/2 pass · set-state gate-pass |

# 작업 로그 — 로드맵 2단계: Pager + 버퍼풀

## 타임라인

| 시각 | 사건 | 결과 |
|---|---|---|
| 2026-11-12 | 사용자 "2단계 진행하자". 브랜치 feat/stage2-pager-buffer-pool(main 8cf93b9). 인터뷰 3회(11문 — 기존 구현·페이지 접근·디렉터리 위치·테스트 저장소·쓰기 정책·오류·영속성·바이트 변환·가드 동시성·카운터·순서), 모두 권장안 선택 | 인터뷰 완료 |
| 2026-11-12 | 리서치: cargo search/info(bytemuck 1.25.2·tempfile 3.27.0·rustc 1.92.0) · 웹(BusTub 버퍼풀·page guard·15-445 디스크 Extendible Hash 3단 페이지) → requirement-spec.md 작성(P1~P6·A1~A2·S1~S5·diff 01~12) | 명세 초안 |
| 2026-11-12 | S1~S5 제안대로 확정. 계획 이해 게이트(하나씩): Q1 "버퍼풀=페이지 캐시, 핀=쓰는 중 내보내기 막음, dirty=변경 유실 막음" · Q2 "PageId, pool.fetch 가드, drop 시 언핀" → (보충 질문) "id×16KB 위치, 고쳤으면 dirty 남음" · Q3 "miss 증가→read 증가, Hash가 랜덤 접근이라 타격 클 듯" → Opus 판정 PASS(보완: 읽기 핀·write 증가·hot set·범위/두 배 차이) | 계획 게이트 통과 |
| 2026-11-12 | 사용자 합의 + lazy 선택 → set-state spec-approved · mode lazy | L1 진입 |

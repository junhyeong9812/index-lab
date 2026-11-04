# 요구사항 명세서 — index-lab 초기화

> 작성일: 2026-11-04 · 작업 폴더: `docs/plans/2026-11-04/init/` · 자명한 작업 — 칸당 축약

## 0. 요구사항 원문

- 원문: "이건 lab폴더 내에 별도 프로젝트로 만들고 위 내용을 리드미로 해서 프로젝트 만들고 깃 초기화까지 가능할까 그리고 이 프로젝트부터 같이 하나씩 진행보자"
- Q/A: 초기 범위 → **README + git init만**(cargo는 로드맵 1단계에서) / 원격 → **GitHub public repo 생성 + push** / lab 목록 → **lab/README.md에 별도 절로 추가** / 위치(lab 구조 확인 후) → **lab/storage-labs/index-lab** — backend-labs-*처럼 스토리지·DB 엔진 실험 묶음 폴더

## 1. 목표·대상 (필수)

`/home/jun/project/lab/storage-labs/index-lab/`에 사용자가 준 README를 그대로 두고 git 초기화(main) → 초기 커밋 → GitHub `junhyeong9812/index-lab`(public) 생성·push. 묶음 폴더 `lab/storage-labs/README.md`(어느 repo에도 속하지 않음) 생성, `/home/jun/project/lab/README.md`에 "스토리지·DB 엔진" 절 추가.

## 2. 경계·불변식 (필수)

README 본문은 사용자가 준 텍스트와 동일(편집 없음).

## 3. 기준소스 (필수)

사용자 메시지의 README 원문.

## 4. 금지영역 (필수)

lab의 다른 실험 폴더 변경 금지. cargo 초기화·소스 생성은 이번 범위 밖.

## 5. 검증 방법 (필수)

README가 원문과 동일(diff), `git log` 초기 커밋, GitHub 저장소에 README 표시(`gh repo view`).

## 6. stakes (필수)

- 판정: **낮음** — 새 폴더·문서 1개, 공개 저장소 생성(내용은 사용자 제공 README).

## 7. 자율성

- [x] auto
- [ ] lazy

## 승인 상태

- [x] 필수 6칸 전부 기입
- [x] 사용자 합의 → SPEC=1 (2026-11-04)
- [ ] 자율성 선택 → MODE 기록

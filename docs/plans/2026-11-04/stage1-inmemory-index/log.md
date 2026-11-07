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

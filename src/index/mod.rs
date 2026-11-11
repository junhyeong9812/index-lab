//! 인덱스 공통 인터페이스 — B+tree와 Hash가 같은 자리에 끼워진다.
//!
//! 키는 `u64`, 값은 `u64` 고정(README 범위: 키 `u64`, 값 고정 길이).

pub mod btree;
pub mod hash;

/// 키 → 값 인덱스.
///
/// 정답지는 `std::collections::BTreeMap<u64, u64>`이며, 모든 메서드는 그와 같은 결과를 내야 한다.
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

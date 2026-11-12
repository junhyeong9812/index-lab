//! Extendible Hash — 디렉터리 → 버킷 2단 구조.
//! 키를 비트 섞기(`mix`)한 해시의 하위 `global_depth` 비트가 디렉터리 칸 번호이고,
//! 칸은 버킷 번호(`BucketId`)를 가리킨다. 여러 칸이 한 버킷을 같이 가리킬 수 있다.

/// 버킷의 배열 번호. 2단계에서 페이지 번호가 된다.
pub type BucketId = usize;

struct Bucket {
    /// 이 버킷이 구분에 쓰는 하위 비트 수(≤ 전역 깊이, I3).
    local_depth: u32,
    /// `keys[i]`의 값 = `values[i]`. 순서 없음.
    keys: Vec<u64>,
    values: Vec<u64>,
}

pub struct Hash {
    /// 길이 = 2^`global_depth`. 칸 i = 해시 하위 비트가 i인 키가 갈 버킷.
    directory: Vec<BucketId>,
    buckets: Vec<Bucket>,
    global_depth: u32,
    bucket_cap: usize,
}

/// splitmix64 마무리 단계 — 비트를 고르게 섞는 전단사 함수(T8).
/// 서로 다른 키는 해시도 다르므로, 분할을 거듭하면 언젠가 반드시 갈라진다.
fn mix(key: u64) -> u64 {
    let mut z = key;
    z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    z ^ (z >> 31)
}

impl Hash {
    /// `bucket_cap` = 버킷 하나에 들어가는 최대 키 수(T9).
    pub fn new(bucket_cap: usize) -> Self {
        assert!(bucket_cap >= 1, "bucket_cap은 1 이상이어야 한다");
        let empty = Bucket {
            local_depth: 0,
            keys: Vec::new(),
            values: Vec::new(),
        };
        // 전역 깊이 0 → 디렉터리 1칸, 버킷 1개.
        Hash {
            directory: vec![0],
            buckets: vec![empty],
            global_depth: 0,
            bucket_cap,
        }
    }

    pub fn get(&self, key: u64) -> Option<u64> {
        let b = &self.buckets[self.directory[self.slot(key)]];
        b.keys.iter().position(|&k| k == key).map(|i| b.values[i])
    }

    /// 같은 키가 있으면 값을 바꾸고 이전 값을 돌려준다(upsert).
    pub fn insert(&mut self, key: u64, value: u64) -> Option<u64> {
        let id = self.directory[self.slot(key)];
        let b = &mut self.buckets[id];
        if let Some(i) = b.keys.iter().position(|&k| k == key) {
            return Some(std::mem::replace(&mut b.values[i], value));
        }
        b.keys.push(key);
        b.values.push(value);
        // 넘치면 나눈다. 키들이 한쪽으로 몰리면 그쪽이 또 넘치므로 반복한다.
        // 넘칠 수 있는 쪽은 언제나 방금 넣은 key가 든 버킷이다(다른 쪽은 그 부분집합이라 용량 이하).
        loop {
            let id = self.directory[self.slot(key)];
            if self.buckets[id].keys.len() <= self.bucket_cap {
                return None;
            }
            self.split(id);
        }
    }

    /// 버킷 `id`를 지역 깊이 d → d+1로 나눈다. 새로 보는 비트 = 해시의 d번째 비트.
    fn split(&mut self, id: BucketId) {
        let d = self.buckets[id].local_depth;
        if d == self.global_depth {
            // 이 버킷이 이미 디렉터리의 모든 비트를 쓰고 있다 → 디렉터리를 두 배로.
            // 뒤쪽 절반 = 앞쪽 복사: 칸 i와 칸 i + 2^전역은 새 비트만 다르니 같은 버킷을 가리킨다.
            self.directory.extend_from_within(..);
            self.global_depth += 1;
        }

        let bit = 1u64 << d;
        let old = &mut self.buckets[id];
        let keys = std::mem::take(&mut old.keys);
        let values = std::mem::take(&mut old.values);
        let mut stay = Bucket {
            local_depth: d + 1,
            keys: Vec::new(),
            values: Vec::new(),
        };
        let mut moved = Bucket {
            local_depth: d + 1,
            keys: Vec::new(),
            values: Vec::new(),
        };
        // 새 비트가 0이면 그대로, 1이면 새 버킷으로.
        for (k, v) in keys.into_iter().zip(values) {
            let to = if mix(k) & bit == 0 {
                &mut stay
            } else {
                &mut moved
            };
            to.keys.push(k);
            to.values.push(v);
        }
        let new_id = self.buckets.len();
        self.buckets[id] = stay;
        self.buckets.push(moved);

        // 이 버킷을 가리키던 2^(전역−d)칸 중 새 비트가 1인 절반을 새 버킷으로 다시 가리킨다.
        // ponytail: 디렉터리 전체를 훑는다(O(2^전역)). 칸만 골라 도는 방식은 분할 비용 측정 때 검토.
        for (slot, b) in self.directory.iter_mut().enumerate() {
            if *b == id && slot as u64 & bit != 0 {
                *b = new_id;
            }
        }
    }

    /// `lo <= key <= hi`를 오름차순으로. 해시는 키 순서를 모르므로 모든 버킷을 훑어 거른 뒤 정렬한다(T10).
    /// `lo > hi`면 거르는 조건을 만족하는 키가 없어 빈 결과가 된다.
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

    /// `range`와 같은 범위를 내림차순으로 — 같은 전체 훑기 후 뒤집는다(T13).
    pub fn range_rev(&self, lo: u64, hi: u64) -> Vec<(u64, u64)> {
        let mut out = self.range(lo, hi);
        out.reverse();
        out
    }

    /// 해시의 하위 `global_depth` 비트 = 디렉터리 칸 번호.
    fn slot(&self, key: u64) -> usize {
        let mask = (1u64 << self.global_depth) - 1;
        (mix(key) & mask) as usize
    }
}

impl super::Index for Hash {
    fn insert(&mut self, key: u64, value: u64) -> Option<u64> {
        Hash::insert(self, key, value)
    }

    fn get(&self, key: u64) -> Option<u64> {
        Hash::get(self, key)
    }

    fn range(&self, lo: u64, hi: u64) -> Vec<(u64, u64)> {
        Hash::range(self, lo, hi)
    }

    fn range_rev(&self, lo: u64, hi: u64) -> Vec<(u64, u64)> {
        Hash::range_rev(self, lo, hi)
    }
}

// ---------- 구조 관찰·검사 (T12: global_depth·bucket_count·check 공개) ----------

impl Hash {
    /// 전역 깊이 — 디렉터리 칸 번호에 쓰는 해시 하위 비트 수(디렉터리 길이 = 2^전역 깊이). T12 공개.
    pub fn global_depth(&self) -> u32 {
        self.global_depth
    }

    /// 버킷 수(디렉터리 칸 수가 아니라 서로 다른 버킷의 수). T12 공개.
    pub fn bucket_count(&self) -> usize {
        self.buckets.len()
    }

    /// I3 구조 불변식 전체 검사(T12 — 테스트와 3단계 실험이 같이 쓴다). 어긋나면 이유와 함께 panic.
    ///
    /// 검사 항목:
    /// - 디렉터리 길이 = 2^전역 깊이, 모든 칸이 있는 버킷을 가리킴
    /// - 모든 버킷 지역 깊이 d ≤ 전역 깊이
    /// - 지역 깊이 d인 버킷을 가리키는 칸 수 = 2^(전역 − d), 그 칸들의 하위 d비트(접미사)가 모두 같음
    /// - 버킷 안 키는 모두 `mix(key)`의 하위 d비트 = 그 접미사(= `get`이 그 버킷을 찾아감)
    /// - `keys`와 `values` 길이 같음, 키 수 ≤ `bucket_cap`, 구조 전체에서 키 중복 없음
    ///
    /// 아무 칸도 가리키지 않는 버킷은 위반으로 본다: I3의 "가리키는 칸 수 = 2^(전역 − d)"는
    /// 모든 버킷에 걸리고 그 값은 1 이상이며, 삭제가 없으니(T4) 버려진 버킷이 생길 이유도 없다.
    /// 그런 버킷이 있으면 `bucket_count`가 실제 쓰이는 버킷 수와 어긋나고, 안에 키가 있으면 조회로 닿지 않는다.
    pub fn check(&self) {
        let g = self.global_depth;
        assert!(
            g < usize::BITS,
            "전역 깊이 {g}: 디렉터리 길이 2^{g}를 usize로 표현할 수 없다"
        );
        assert_eq!(
            self.directory.len(),
            1usize << g,
            "디렉터리 길이 {}가 2^전역 깊이(2^{g})와 다르다",
            self.directory.len()
        );

        // 버킷마다 자기를 가리키는 칸 목록.
        let mut refs: Vec<Vec<usize>> = vec![Vec::new(); self.buckets.len()];
        for (s, &id) in self.directory.iter().enumerate() {
            assert!(
                id < self.buckets.len(),
                "디렉터리 칸 {s}: 없는 버킷 {id}를 가리킨다(버킷 {}개)",
                self.buckets.len()
            );
            refs[id].push(s);
        }

        let mut seen = std::collections::HashSet::new();
        for (id, b) in self.buckets.iter().enumerate() {
            let d = b.local_depth;
            assert!(d <= g, "버킷 {id}: 지역 깊이 {d} > 전역 깊이 {g}");
            let slots = &refs[id];
            assert!(
                !slots.is_empty(),
                "버킷 {id}: 어떤 디렉터리 칸도 가리키지 않는다"
            );
            assert_eq!(
                slots.len(),
                1usize << (g - d),
                "버킷 {id}(지역 깊이 {d}): 가리키는 칸 수가 2^(전역 {g} − 지역 {d})이 아니다 — 칸 {slots:?}"
            );
            let mask = (1u64 << d) - 1;
            let suffix = slots[0] as u64 & mask;
            for &s in slots {
                assert_eq!(
                    s as u64 & mask,
                    suffix,
                    "버킷 {id}(지역 깊이 {d}): 가리키는 칸 {slots:?}의 하위 {d}비트가 서로 다르다"
                );
            }
            assert_eq!(
                b.keys.len(),
                b.values.len(),
                "버킷 {id}: 키 {}개와 값 {}개의 수가 다르다",
                b.keys.len(),
                b.values.len()
            );
            assert!(
                b.keys.len() <= self.bucket_cap,
                "버킷 {id}: 키 {}개가 용량 {}을 넘는다",
                b.keys.len(),
                self.bucket_cap
            );
            for &k in &b.keys {
                assert_eq!(
                    mix(k) & mask,
                    suffix,
                    "버킷 {id}(지역 깊이 {d}, 접미사 {suffix:#b}): 키 {k}의 해시 하위 {d}비트가 접미사와 다르다"
                );
                assert!(
                    seen.insert(k),
                    "키 {k}가 버킷 둘 이상(또는 한 버킷에 두 번) 들어 있다"
                );
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::index::Index;
    use proptest::prelude::*;
    use std::collections::BTreeMap;

    // ---------- 테스트 보조 ----------

    /// `mix(k)`의 하위 `bits`비트가 `suffix`인 키를 0부터 차례로 찾아 `n`개(`skip`에 있는 키는 건너뜀).
    /// 분할이 어느 쪽으로 갈지 테스트가 정할 수 있게 한다 — 해시 하위 비트를 무차별 대입으로 맞춘다.
    fn keys_with_suffix(bits: u32, suffix: u64, n: usize, skip: &[u64]) -> Vec<u64> {
        let mask = (1u64 << bits) - 1;
        (0u64..)
            .filter(|&k| mix(k) & mask == suffix && !skip.contains(&k))
            .take(n)
            .collect()
    }

    /// 구조에 들어 있는 (키, 값) 전체 — 정답지와 비교용.
    fn entries(h: &Hash) -> Vec<(u64, u64)> {
        h.range(0, u64::MAX)
    }

    /// 키 `k`가 들어 있는 버킷 번호(구조를 직접 훑는다 — `get` 경로와 무관하게).
    fn bucket_of(h: &Hash, k: u64) -> BucketId {
        h.buckets
            .iter()
            .position(|b| b.keys.contains(&k))
            .unwrap_or_else(|| panic!("키 {k}가 어느 버킷에도 없다"))
    }

    /// 변조 테스트용 — 용량 2로 키 40개를 넣어 분할이 여러 번 일어난 정상 구조.
    fn sample() -> Hash {
        let mut h = Hash::new(2);
        for k in 0..40 {
            h.insert(k, k * 10);
        }
        h.check();
        assert!(
            h.global_depth() >= 3,
            "변조 대상은 분할이 충분히 일어난 구조여야 한다"
        );
        h
    }

    // ---------- 단위 테스트 ----------

    #[test]
    fn new_is_empty() {
        let h = Hash::new(4);
        h.check();
        assert_eq!(h.global_depth(), 0);
        assert_eq!(h.bucket_count(), 1);
        assert_eq!(h.get(0), None);
        assert_eq!(h.get(u64::MAX), None);
        assert!(h.range(0, u64::MAX).is_empty());
        assert!(h.range_rev(0, u64::MAX).is_empty());
    }

    #[test]
    #[should_panic]
    fn new_with_cap_0_panics() {
        let _ = Hash::new(0);
    }

    #[test]
    fn insert_get_without_split() {
        let mut h = Hash::new(4);
        assert_eq!(h.insert(5, 50), None);
        assert_eq!(h.insert(1, 10), None);
        assert_eq!(h.insert(9, 90), None);
        h.check();
        // 용량 안이면 분할 없음.
        assert_eq!(h.global_depth(), 0);
        assert_eq!(h.bucket_count(), 1);
        assert_eq!(h.get(5), Some(50));
        assert_eq!(h.get(1), Some(10));
        assert_eq!(h.get(9), Some(90));
        assert_eq!(h.get(2), None);
        assert_eq!(entries(&h), vec![(1, 10), (5, 50), (9, 90)]);
    }

    #[test]
    fn upsert_returns_old_value() {
        let mut h = Hash::new(2);
        assert_eq!(h.insert(7, 1), None);
        assert_eq!(h.insert(7, 2), Some(1));
        assert_eq!(h.insert(7, 3), Some(2));
        assert_eq!(h.get(7), Some(3));
        assert_eq!(entries(&h), vec![(7, 3)]);
        h.check();
    }

    /// 꽉 찬 버킷에 이미 있는 키를 다시 넣으면 값만 바뀌고 분할은 일어나지 않는다
    /// (새 키가 들어갈 자리가 없을 때만 분할 — 갱신은 키 수를 늘리지 않는다).
    #[test]
    fn upsert_into_full_bucket_does_not_split() {
        let mut h = Hash::new(2);
        h.insert(100, 1);
        h.insert(200, 2);
        assert_eq!((h.global_depth(), h.bucket_count()), (0, 1));
        assert_eq!(h.insert(100, 11), Some(1));
        assert_eq!(h.insert(200, 22), Some(2));
        assert_eq!((h.global_depth(), h.bucket_count()), (0, 1));
        assert_eq!(entries(&h), vec![(100, 11), (200, 22)]);
        h.check();
    }

    /// 지역 깊이 = 전역 깊이인 버킷이 넘치면 디렉터리가 두 배가 된다.
    #[test]
    fn split_with_local_eq_global_doubles_directory() {
        let mut h = Hash::new(2);
        // 하위 1비트: 0, 1, 0 — 세 번째 삽입에서 버킷(지역 0 = 전역 0)이 넘친다.
        // 하위 2비트로는 0쪽 두 키가 00·10으로 갈리게 골라 분할이 한 번으로 끝나게 한다.
        let a = keys_with_suffix(2, 0b00, 1, &[])[0];
        let b = keys_with_suffix(1, 0b1, 1, &[])[0];
        let c = keys_with_suffix(2, 0b10, 1, &[])[0];
        assert_eq!(h.insert(a, 1), None);
        assert_eq!(h.insert(b, 2), None);
        assert_eq!((h.global_depth(), h.bucket_count()), (0, 1));
        assert_eq!(h.insert(c, 3), None);
        h.check();
        assert_eq!(h.global_depth(), 1, "디렉터리가 1칸 → 2칸");
        assert_eq!(h.directory.len(), 2);
        assert_eq!(h.bucket_count(), 2);
        assert_eq!(bucket_of(&h, a), bucket_of(&h, c));
        assert_ne!(bucket_of(&h, a), bucket_of(&h, b));
        assert_eq!((h.get(a), h.get(b), h.get(c)), (Some(1), Some(2), Some(3)));
    }

    /// 지역 깊이 < 전역 깊이인 버킷이 넘치면 디렉터리 길이는 그대로, 그 버킷만 갈라진다.
    #[test]
    fn split_with_local_lt_global_keeps_directory() {
        let mut h = Hash::new(2);
        // 1) 하위 2비트 00·10·00(셋째는 3비트에서 갈리게) + 하위 1비트 1 하나.
        let a = keys_with_suffix(3, 0b000, 1, &[])[0];
        let c = keys_with_suffix(2, 0b10, 1, &[])[0];
        let d = keys_with_suffix(3, 0b100, 1, &[])[0];
        let b = keys_with_suffix(2, 0b01, 1, &[])[0];
        for (i, &k) in [a, b, c].iter().enumerate() {
            assert_eq!(h.insert(k, i as u64), None);
        }
        h.check();
        assert_eq!((h.global_depth(), h.bucket_count()), (1, 2));
        // 2) 0쪽 버킷(지역 1 = 전역 1)에 셋째 키 → 디렉터리 두 배(전역 2).
        //    a·d는 하위 2비트 00, c는 10 → 00 버킷 2개, 10 버킷 1개로 갈린다.
        assert_eq!(h.insert(d, 3), None);
        h.check();
        assert_eq!((h.global_depth(), h.bucket_count()), (2, 3));
        // 1쪽 버킷은 지역 깊이 1로 남아 칸 01·11 두 개가 함께 가리킨다.
        let one = bucket_of(&h, b);
        assert_eq!(h.buckets[one].local_depth, 1);
        assert_eq!(h.directory[0b01], one);
        assert_eq!(h.directory[0b11], one);
        // 3) 1쪽 버킷(지역 1 < 전역 2)을 넘치게 — 하위 2비트 11 키 두 개 추가.
        let e = keys_with_suffix(2, 0b11, 2, &[]);
        assert_eq!(h.insert(e[0], 4), None);
        h.check();
        assert_eq!(h.global_depth(), 2);
        assert_eq!(h.insert(e[1], 5), None);
        h.check();
        assert_eq!(
            h.global_depth(),
            2,
            "지역 < 전역 분할은 디렉터리를 늘리지 않는다"
        );
        assert_eq!(h.directory.len(), 4);
        assert_eq!(h.bucket_count(), 4);
        // 칸 01은 b만, 칸 11은 e 두 개를 가진 서로 다른 버킷을 가리킨다.
        assert_ne!(h.directory[0b01], h.directory[0b11]);
        assert_eq!(h.directory[0b01], bucket_of(&h, b));
        assert_eq!(h.directory[0b11], bucket_of(&h, e[0]));
        assert_eq!(h.directory[0b11], bucket_of(&h, e[1]));
        for (k, v) in [(a, 0), (b, 1), (c, 2), (d, 3), (e[0], 4), (e[1], 5)] {
            assert_eq!(h.get(k), Some(v));
        }
    }

    /// 하위 비트를 많이 공유하는 키들은 한 삽입 안에서 분할이 거듭되어야 갈라진다.
    #[test]
    fn repeated_split_when_keys_share_low_bits() {
        let mut h = Hash::new(2);
        // 셋 다 하위 3비트 000. 넷째 비트에서 x·y(0)와 z(1)가 갈리고, x·y는 다섯째 비트에서 갈린다.
        let x = keys_with_suffix(5, 0b00000, 1, &[])[0];
        let y = keys_with_suffix(5, 0b10000, 1, &[])[0];
        let z = keys_with_suffix(4, 0b1000, 1, &[])[0];
        h.insert(x, 1);
        h.insert(y, 2);
        assert_eq!((h.global_depth(), h.bucket_count()), (0, 1));
        h.insert(z, 3);
        h.check();
        // 깊이 1·2·3에서는 셋이 한쪽으로 몰려 계속 넘치고, 깊이 4에서 z가 떨어져 나간다.
        // 분할 한 번 = 버킷 하나 추가 → 분할 4번, 버킷 5개.
        assert_eq!(h.global_depth(), 4);
        assert_eq!(h.bucket_count(), 5);
        assert_eq!(bucket_of(&h, x), bucket_of(&h, y));
        assert_ne!(bucket_of(&h, x), bucket_of(&h, z));
        assert_eq!((h.get(x), h.get(y), h.get(z)), (Some(1), Some(2), Some(3)));
    }

    #[test]
    fn many_inserts_cap_1_and_cap_2() {
        for cap in [1, 2] {
            let mut h = Hash::new(cap);
            for k in 0..300u64 {
                assert_eq!(h.insert(k, !k), None);
            }
            h.check();
            assert!(h.bucket_count() * cap >= 300);
            for k in 0..300u64 {
                assert_eq!(h.get(k), Some(!k));
            }
            assert_eq!(h.get(300), None);
            let want: Vec<(u64, u64)> = (0..300u64).map(|k| (k, !k)).collect();
            assert_eq!(entries(&h), want);
        }
    }

    #[test]
    fn range_edges() {
        let mut h = Hash::new(2);
        for k in [0u64, 1, 5, 10, 11, 1000, u64::MAX - 1, u64::MAX] {
            h.insert(k, k ^ 1);
        }
        h.check();
        // lo > hi → 빈 결과.
        assert!(h.range(10, 5).is_empty());
        assert!(h.range_rev(10, 5).is_empty());
        assert!(h.range(u64::MAX, 0).is_empty());
        // 한 점.
        assert_eq!(h.range(5, 5), vec![(5, 4)]);
        assert_eq!(h.range_rev(5, 5), vec![(5, 4)]);
        assert!(h.range(6, 6).is_empty());
        // 양끝 포함.
        assert_eq!(h.range(1, 10), vec![(1, 0), (5, 4), (10, 11)]);
        assert_eq!(h.range(2, 9), vec![(5, 4)]);
        // u64::MAX 키.
        assert_eq!(h.range(u64::MAX, u64::MAX), vec![(u64::MAX, u64::MAX ^ 1)]);
        assert_eq!(
            h.range(u64::MAX - 1, u64::MAX),
            vec![(u64::MAX - 1, u64::MAX), (u64::MAX, u64::MAX ^ 1)]
        );
        assert_eq!(h.get(u64::MAX), Some(u64::MAX ^ 1));
        // 전체.
        let all = h.range(0, u64::MAX);
        assert_eq!(all.len(), 8);
        assert!(all.windows(2).all(|w| w[0].0 < w[1].0));
        let mut rev = h.range_rev(0, u64::MAX);
        rev.reverse();
        assert_eq!(rev, all);
        // u64::MAX 키 upsert.
        assert_eq!(h.insert(u64::MAX, 7), Some(u64::MAX ^ 1));
        assert_eq!(
            h.range_rev(u64::MAX - 1, u64::MAX),
            vec![(u64::MAX, 7), (u64::MAX - 1, u64::MAX)]
        );
        h.check();
    }

    #[test]
    fn works_through_dyn_index() {
        let mut h = Hash::new(2);
        let idx: &mut dyn Index = &mut h;
        for k in 0..30 {
            assert_eq!(idx.insert(k, k + 1), None);
        }
        assert_eq!(idx.insert(7, 0), Some(8));
        assert_eq!(idx.get(7), Some(0));
        assert_eq!(idx.get(30), None);
        assert_eq!(idx.range(5, 8), vec![(5, 6), (6, 7), (7, 0), (8, 9)]);
        assert_eq!(idx.range_rev(5, 8), vec![(8, 9), (7, 0), (6, 7), (5, 6)]);
        assert!(idx.range(8, 5).is_empty());
        h.check();

        // 3단계 러너처럼 Box<dyn Index>로.
        let mut boxed: Box<dyn Index> = Box::new(Hash::new(3));
        assert_eq!(boxed.insert(u64::MAX, 1), None);
        assert_eq!(boxed.insert(0, 2), None);
        assert_eq!(boxed.insert(u64::MAX, 3), Some(1));
        assert_eq!(boxed.range(0, u64::MAX), vec![(0, 2), (u64::MAX, 3)]);
        assert_eq!(boxed.range_rev(0, u64::MAX), vec![(u64::MAX, 3), (0, 2)]);
    }

    // ---------- check() 자체 검증 — 구조를 일부러 망가뜨리면 잡아야 한다 ----------

    #[test]
    #[should_panic(expected = "의 해시 하위")]
    fn check_catches_wrong_directory_pointer() {
        let mut h = sample();
        // 키가 있고 지역 깊이 = 전역 깊이인(칸 하나만 받는) 서로 다른 두 버킷의 칸을 맞바꾼다
        // — 버킷마다 칸 수는 그대로 1이라 키 위치로만 드러난다.
        let g = h.global_depth;
        let full: Vec<usize> = (0..h.directory.len())
            .filter(|&s| {
                let b = &h.buckets[h.directory[s]];
                b.local_depth == g && !b.keys.is_empty()
            })
            .collect();
        assert!(full.len() >= 2, "sample 전제: 맞바꿀 칸 둘");
        let (s1, s2) = (full[0], full[1]);
        h.directory.swap(s1, s2);
        h.check();
    }

    #[test]
    #[should_panic(expected = "가리키는 칸 수")]
    fn check_catches_pointer_to_wrong_bucket_count() {
        let mut h = sample();
        // 한 칸을 다른 버킷으로 돌린다 — 원래 버킷의 칸 수가 모자라진다.
        let s = h.directory.len() - 1;
        h.directory[s] = (h.directory[s] + 1) % h.buckets.len();
        h.check();
    }

    #[test]
    #[should_panic(expected = "없는 버킷")]
    fn check_catches_dangling_pointer() {
        let mut h = sample();
        h.directory[0] = h.buckets.len();
        h.check();
    }

    #[test]
    #[should_panic(expected = "> 전역 깊이")]
    fn check_catches_local_depth_over_global() {
        let mut h = sample();
        let id = h.directory[0];
        h.buckets[id].local_depth = h.global_depth + 1;
        h.check();
    }

    #[test]
    #[should_panic(expected = "의 해시 하위")]
    fn check_catches_key_in_wrong_bucket() {
        let mut h = sample();
        // 키 하나를 지역 깊이 ≥ 1인 다른 접미사의 버킷으로 옮긴다(용량은 넘지 않게 빈 자리 있는 곳으로).
        let k = 0u64;
        let from = bucket_of(&h, k);
        let to = (0..h.buckets.len())
            .find(|&id| {
                id != from
                    && h.buckets[id].keys.len() < h.bucket_cap
                    && h.buckets[id].local_depth >= 1
            })
            .expect("자리 있는 다른 버킷");
        let i = h.buckets[from].keys.iter().position(|&x| x == k).unwrap();
        let v = h.buckets[from].values.remove(i);
        h.buckets[from].keys.remove(i);
        h.buckets[to].keys.push(k);
        h.buckets[to].values.push(v);
        h.check();
    }

    #[test]
    #[should_panic(expected = "디렉터리 길이")]
    fn check_catches_directory_length() {
        let mut h = sample();
        h.directory.pop();
        h.check();
    }

    #[test]
    #[should_panic(expected = "용량")]
    fn check_catches_overfull_bucket() {
        let mut h = sample();
        let id = h.directory[0];
        // 같은 접미사를 가진 새 키를 용량을 넘겨 밀어 넣는다.
        let d = h.buckets[id].local_depth;
        let suffix = 0; // 칸 0을 가리키는 버킷의 접미사는 0.
        let existing: Vec<u64> = h.buckets.iter().flat_map(|b| b.keys.clone()).collect();
        let need = h.bucket_cap + 1 - h.buckets[id].keys.len();
        for k in keys_with_suffix(d, suffix, need, &existing) {
            h.buckets[id].keys.push(k);
            h.buckets[id].values.push(0);
        }
        h.check();
    }

    #[test]
    #[should_panic(expected = "수가 다르다")]
    fn check_catches_keys_values_length_mismatch() {
        let mut h = sample();
        let id = bucket_of(&h, 0);
        h.buckets[id].values.push(1);
        h.check();
    }

    #[test]
    #[should_panic(expected = "버킷 둘 이상")]
    fn check_catches_duplicate_key() {
        let mut h = Hash::new(4);
        h.insert(3, 30);
        h.check();
        h.buckets[0].keys.push(3);
        h.buckets[0].values.push(31);
        h.check();
    }

    #[test]
    #[should_panic(expected = "어떤 디렉터리 칸도")]
    fn check_catches_unreferenced_bucket() {
        let mut h = sample();
        h.buckets.push(Bucket {
            local_depth: 0,
            keys: Vec::new(),
            values: Vec::new(),
        });
        h.check();
    }

    // ---------- 속성 테스트 (I1·I3·I4, 정답지 BTreeMap) ----------

    /// 좁은 키 공간(중복·upsert 유도) 위주 + u64 끝 근처 + 전체 u64.
    fn key() -> impl Strategy<Value = u64> {
        prop_oneof![
            6 => 0u64..200,
            1 => (u64::MAX - 20)..=u64::MAX,
            2 => any::<u64>(),
        ]
    }

    fn ops(max_len: usize) -> impl Strategy<Value = Vec<(u64, u64)>> {
        prop::collection::vec((key(), any::<u64>()), 0..max_len)
    }

    /// `[lo, hi]` 정답(lo > hi면 빈 결과 — BTreeMap::range는 이때 panic하므로 막는다).
    fn model_range(map: &BTreeMap<u64, u64>, lo: u64, hi: u64) -> Vec<(u64, u64)> {
        if lo > hi {
            return Vec::new();
        }
        map.range(lo..=hi).map(|(&k, &v)| (k, v)).collect()
    }

    fn model_range_rev(map: &BTreeMap<u64, u64>, lo: u64, hi: u64) -> Vec<(u64, u64)> {
        if lo > hi {
            return Vec::new();
        }
        map.range(lo..=hi).rev().map(|(&k, &v)| (k, v)).collect()
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(256))]

        #[test]
        fn matches_btreemap(
            cap in 2usize..=4,
            ops in ops(300),
            queries in prop::collection::vec((key(), key()), 0..32),
        ) {
            let mut h = Hash::new(cap);
            let mut map = BTreeMap::new();
            for &(k, v) in &ops {
                prop_assert_eq!(h.insert(k, v), map.insert(k, v), "insert({}, {})", k, v);
            }
            h.check();

            let want: Vec<(u64, u64)> = map.iter().map(|(&k, &v)| (k, v)).collect();
            prop_assert_eq!(h.range(0, u64::MAX), want);

            for &(k, _) in &ops {
                prop_assert_eq!(h.get(k), map.get(&k).copied());
            }
            for &(lo, hi) in &queries {
                prop_assert_eq!(h.get(lo), map.get(&lo).copied());
                prop_assert_eq!(h.range(lo, hi), model_range(&map, lo, hi), "range({}, {})", lo, hi);
                prop_assert_eq!(
                    h.range_rev(lo, hi),
                    model_range_rev(&map, lo, hi),
                    "range_rev({}, {})", lo, hi
                );
            }

            // 같은 대조를 trait 객체로 한 번 더.
            let idx: &dyn Index = &h;
            for &(lo, hi) in &queries {
                prop_assert_eq!(idx.get(hi), map.get(&hi).copied());
                prop_assert_eq!(idx.range(lo, hi), model_range(&map, lo, hi));
                prop_assert_eq!(idx.range_rev(lo, hi), model_range_rev(&map, lo, hi));
            }
        }

        /// 삽입 한 번마다 구조 검사 — 분할 직후의 중간 상태도 I3를 지키는지.
        /// 삭제가 없으니(T4) 전역 깊이·버킷 수는 줄지 않고, upsert는 구조를 바꾸지 않는다.
        #[test]
        fn structure_holds_after_every_insert(
            cap in 2usize..=4,
            ops in ops(120),
        ) {
            let mut h = Hash::new(cap);
            let mut map = BTreeMap::new();
            for &(k, v) in &ops {
                let before = (h.global_depth(), h.bucket_count());
                let old = map.insert(k, v);
                prop_assert_eq!(h.insert(k, v), old, "insert({}, {})", k, v);
                h.check();
                let after = (h.global_depth(), h.bucket_count());
                prop_assert!(after.0 >= before.0 && after.1 >= before.1, "깊이·버킷 수가 줄었다: {:?} → {:?}", before, after);
                if old.is_some() {
                    prop_assert_eq!(after, before, "upsert({}) 가 구조를 바꿨다", k);
                }
                prop_assert_eq!(h.get(k), Some(v));
            }
            prop_assert!(h.bucket_count() * cap >= map.len());
            let want: Vec<(u64, u64)> = map.iter().rev().map(|(&k, &v)| (k, v)).collect();
            prop_assert_eq!(h.range_rev(0, u64::MAX), want);
        }
    }
}

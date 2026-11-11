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

    /// 해시의 하위 `global_depth` 비트 = 디렉터리 칸 번호.
    fn slot(&self, key: u64) -> usize {
        let mask = (1u64 << self.global_depth) - 1;
        (mix(key) & mask) as usize
    }
}

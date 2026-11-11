//! Extendible Hash — 디렉터리 → 버킷 2단 구조.
//! 키를 비트 섞기(`mix`)한 해시의 하위 `global_depth` 비트가 디렉터리 칸 번호이고,
//! 칸은 버킷 번호(`BucketId`)를 가리킨다. 여러 칸이 한 버킷을 같이 가리킬 수 있다.

/// 버킷의 배열 번호. 2단계에서 페이지 번호가 된다.
pub type BucketId = usize;

struct Bucket {
    /// 이 버킷이 구분에 쓰는 하위 비트 수(≤ 전역 깊이, I3).
    #[expect(dead_code, reason = "diff 08 분할에서 읽는다")]
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
    #[expect(dead_code, reason = "diff 08 넘침 판정에서 읽는다")]
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

    /// 해시의 하위 `global_depth` 비트 = 디렉터리 칸 번호.
    fn slot(&self, key: u64) -> usize {
        let mask = (1u64 << self.global_depth) - 1;
        (mix(key) & mask) as usize
    }
}

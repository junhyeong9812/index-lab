//! 2단계 바닥층: 16KB 페이지 단위 저장소와 그 위의 Pager(실제 I/O 계수).

pub mod file;
pub mod pager;
pub mod pool;

use std::io;

use bytemuck::{Pod, Zeroable};

/// 페이지 크기. 디스크 I/O·버퍼풀 프레임·인덱스 노드의 단위.
pub const PAGE_SIZE: usize = 16 * 1024;

/// 페이지 번호. 파일 안 위치 = `id × PAGE_SIZE`.
pub type PageId = u64;

/// 페이지 한 장의 바이트. `align(8)` — 페이지 구조체(`u64` 배열)로 그대로 캐스팅하려면 정렬이 맞아야 한다.
#[repr(C, align(8))]
#[derive(Clone, Copy, Pod, Zeroable)]
pub struct Page(pub [u8; PAGE_SIZE]);

/// 페이지 단위로 읽고 쓰는 저장소. 메모리(테스트)와 파일 두 구현이 같은 의미를 가진다.
pub trait Storage {
    /// 지금 저장소에 있는 페이지 수. 페이지 `0..page_count()`만 읽을 수 있다.
    fn page_count(&self) -> io::Result<u64>;
    fn read_page(&mut self, id: PageId, page: &mut Page) -> io::Result<()>;
    /// 끝 뒤에 쓰면 저장소가 `id + 1`장으로 자라고, 그 사이 페이지는 0으로 채워진다.
    /// 버퍼풀은 할당 순서와 다른 순서로 페이지를 내보내므로(새 페이지 6이 5보다 먼저 쓰일 수 있다) 구멍을 허용한다.
    fn write_page(&mut self, id: PageId, page: &Page) -> io::Result<()>;
}

/// 메모리 저장소 — 테스트용. 파일 없이 같은 의미.
#[derive(Default)]
pub struct MemStorage {
    pages: Vec<Box<Page>>,
}

impl MemStorage {
    pub fn new() -> Self {
        Self::default()
    }
}

impl Storage for MemStorage {
    fn page_count(&self) -> io::Result<u64> {
        Ok(self.pages.len() as u64)
    }

    fn read_page(&mut self, id: PageId, page: &mut Page) -> io::Result<()> {
        let src = self
            .pages
            .get(id as usize)
            .ok_or_else(|| out_of_range(id, self.pages.len()))?;
        *page = **src;
        Ok(())
    }

    fn write_page(&mut self, id: PageId, page: &Page) -> io::Result<()> {
        let i = id as usize;
        if i >= self.pages.len() {
            self.pages.resize_with(i + 1, || Box::new(Page::zeroed()));
        }
        *self.pages[i] = *page;
        Ok(())
    }
}

fn out_of_range(id: PageId, count: usize) -> io::Error {
    io::Error::new(
        io::ErrorKind::InvalidInput,
        format!("page {id} out of range (page_count {count})"),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    // A1 스모크: 헤더 + 긴 u64 배열 두 개로 정확히 16KB인 페이지 구조체가 Pod로 derive되고,
    // Page 바이트와 서로 캐스팅된다(패딩이 있으면 derive가 컴파일 오류).
    #[repr(C)]
    #[derive(Clone, Copy, Pod, Zeroable)]
    struct LeafLike {
        count: u64,
        next: u64,
        keys: [u64; 1023],
        values: [u64; 1023],
    }
    const _: () = assert!(size_of::<LeafLike>() == PAGE_SIZE);

    #[test]
    fn page_struct_round_trips_through_bytes() {
        let mut page = Page::zeroed();
        let leaf: &mut LeafLike = bytemuck::cast_mut(&mut page);
        leaf.count = 2;
        leaf.keys[1022] = 7;
        leaf.values[1022] = 70;
        assert_eq!(&page.0[0..8], &2u64.to_ne_bytes());
        let back: &LeafLike = bytemuck::cast_ref(&page);
        assert_eq!((back.count, back.keys[1022], back.values[1022]), (2, 7, 70));
    }

    #[test]
    fn mem_storage_grows_past_end_with_zero_pages() {
        let mut s = MemStorage::new();
        let mut p = Page::zeroed();
        p.0[0] = 9;
        s.write_page(1, &p).unwrap();
        assert_eq!(s.page_count().unwrap(), 2);
        let mut q = Page::zeroed();
        q.0[0] = 1;
        s.read_page(0, &mut q).unwrap();
        assert_eq!(q.0[0], 0, "사이 페이지는 0");
        s.read_page(1, &mut q).unwrap();
        assert_eq!(q.0[0], 9);
        assert!(s.read_page(2, &mut q).is_err());
    }
}

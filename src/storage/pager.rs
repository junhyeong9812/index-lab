//! Pager — 저장소 앞에서 페이지 할당과 실제 I/O 횟수를 센다(P1).

use std::io;

use super::{Page, PageId, Storage};

pub struct Pager<S: Storage> {
    storage: S,
    /// 다음에 할당할 페이지 번호. 할당만으로는 I/O가 없고, 첫 `write`에서 저장소가 자란다.
    next: PageId,
    reads: u64,
    writes: u64,
}

impl<S: Storage> Pager<S> {
    pub fn new(storage: S) -> io::Result<Self> {
        let next = storage.page_count()?;
        Ok(Self {
            storage,
            next,
            reads: 0,
            writes: 0,
        })
    }

    /// 새 페이지 번호를 하나 준다. 디스크에 닿지 않는다 — 버퍼풀이 내용을 채워 나중에 `write`한다.
    pub fn allocate(&mut self) -> PageId {
        let id = self.next;
        self.next += 1;
        id
    }

    pub fn page_count(&self) -> u64 {
        self.next
    }

    /// 성공한 읽기만 센다.
    pub fn read(&mut self, id: PageId, page: &mut Page) -> io::Result<()> {
        self.storage.read_page(id, page)?;
        self.reads += 1;
        Ok(())
    }

    /// 성공한 쓰기만 센다.
    pub fn write(&mut self, id: PageId, page: &Page) -> io::Result<()> {
        if id >= self.next {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                format!("page {id} not allocated"),
            ));
        }
        self.storage.write_page(id, page)?;
        self.writes += 1;
        Ok(())
    }

    pub fn reads(&self) -> u64 {
        self.reads
    }

    pub fn writes(&self) -> u64 {
        self.writes
    }
}

#[cfg(test)]
mod tests {
    use bytemuck::Zeroable;

    use super::*;
    use crate::storage::MemStorage;

    #[test]
    fn counts_only_successful_io() {
        let mut pager = Pager::new(MemStorage::new()).unwrap();
        let mut p = Page::zeroed();
        assert!(pager.write(0, &p).is_err(), "할당 전 쓰기");
        let a = pager.allocate();
        let b = pager.allocate();
        assert_eq!((a, b, pager.writes()), (0, 1, 0), "할당은 I/O 아님");
        p.0[5] = 1;
        pager.write(a, &p).unwrap();
        pager.write(b, &p).unwrap();
        pager.write(a, &p).unwrap();
        let mut q = Page::zeroed();
        pager.read(b, &mut q).unwrap();
        assert!(pager.read(2, &mut q).is_err());
        assert_eq!((pager.reads(), pager.writes(), q.0[5]), (1, 3, 1));
    }
}

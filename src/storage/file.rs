//! 파일 저장소 — 페이지 `id`는 파일의 `id × PAGE_SIZE` 바이트 위치. `read_at`/`write_at`(pread/pwrite)로 위치를 직접 지정한다.

use std::fs::{File, OpenOptions};
use std::io;
use std::os::unix::fs::FileExt;
use std::path::Path;

use super::{PAGE_SIZE, Page, PageId, Storage};

pub struct FileStorage {
    file: File,
    /// 파일에 있는 페이지 수. 열 때 파일 길이로 정하고, 끝에 붙일 때 늘린다.
    pages: u64,
}

impl FileStorage {
    /// 있으면 열고 없으면 만든다. 기존 내용은 지우지 않는다 — 다시 열기(P5)의 전제.
    pub fn open(path: impl AsRef<Path>) -> io::Result<Self> {
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(path)?;
        let len = file.metadata()?.len();
        if len % PAGE_SIZE as u64 != 0 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!("file length {len} is not a multiple of PAGE_SIZE"),
            ));
        }
        Ok(Self {
            file,
            pages: len / PAGE_SIZE as u64,
        })
    }
}

fn offset(id: PageId) -> u64 {
    id * PAGE_SIZE as u64
}

impl Storage for FileStorage {
    fn page_count(&self) -> io::Result<u64> {
        Ok(self.pages)
    }

    fn read_page(&mut self, id: PageId, page: &mut Page) -> io::Result<()> {
        if id >= self.pages {
            return Err(out_of_range(id, self.pages));
        }
        self.file.read_exact_at(&mut page.0, offset(id))
    }

    fn write_page(&mut self, id: PageId, page: &Page) -> io::Result<()> {
        if id > self.pages {
            return Err(out_of_range(id, self.pages));
        }
        self.file.write_all_at(&page.0, offset(id))?;
        if id == self.pages {
            self.pages += 1;
        }
        Ok(())
    }
}

fn out_of_range(id: PageId, count: u64) -> io::Error {
    io::Error::new(
        io::ErrorKind::InvalidInput,
        format!("page {id} out of range (page_count {count})"),
    )
}

#[cfg(test)]
mod tests {
    use bytemuck::Zeroable;

    use super::*;
    use crate::storage::MemStorage;
    use crate::storage::pager::Pager;

    fn page_with(byte: u8) -> Page {
        let mut p = Page::zeroed();
        p.0[0] = byte;
        p.0[PAGE_SIZE - 1] = byte;
        p
    }

    /// 같은 연산열을 돌리고 (각 페이지 첫·끝 바이트, reads, writes)를 돌려준다.
    fn run<S: Storage>(storage: S) -> (Vec<(u8, u8)>, u64, u64) {
        let mut pager = Pager::new(storage).unwrap();
        let ids: Vec<_> = (0..3).map(|_| pager.allocate()).collect();
        for &id in &ids {
            pager.write(id, &page_with(id as u8 + 10)).unwrap();
        }
        pager.write(ids[1], &page_with(99)).unwrap();
        let mut p = Page::zeroed();
        assert!(pager.read(3, &mut p).is_err());
        let seen = ids
            .iter()
            .map(|&id| {
                pager.read(id, &mut p).unwrap();
                (p.0[0], p.0[PAGE_SIZE - 1])
            })
            .collect();
        (seen, pager.reads(), pager.writes())
    }

    #[test]
    fn file_and_memory_agree() {
        let dir = tempfile::tempdir().unwrap();
        let file = run(FileStorage::open(dir.path().join("a.db")).unwrap());
        let mem = run(MemStorage::new());
        assert_eq!(file, mem);
        assert_eq!(file, (vec![(10, 10), (99, 99), (12, 12)], 3, 4));
    }

    #[test]
    fn contents_survive_reopen() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("a.db");
        {
            let mut s = FileStorage::open(&path).unwrap();
            s.write_page(0, &page_with(7)).unwrap();
            s.write_page(1, &page_with(8)).unwrap();
        } // 닫힘
        assert_eq!(
            std::fs::metadata(&path).unwrap().len(),
            2 * PAGE_SIZE as u64
        );
        let mut s = FileStorage::open(&path).unwrap();
        assert_eq!(s.page_count().unwrap(), 2);
        let mut p = Page::zeroed();
        s.read_page(1, &mut p).unwrap();
        assert_eq!((p.0[0], p.0[PAGE_SIZE - 1]), (8, 8));
        // 다시 연 Pager는 기존 페이지 뒤부터 할당한다.
        assert_eq!(Pager::new(s).unwrap().allocate(), 2);
    }

    #[test]
    fn rejects_holes_and_torn_files() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("a.db");
        let mut s = FileStorage::open(&path).unwrap();
        assert!(s.write_page(1, &page_with(1)).is_err(), "구멍 쓰기");
        assert_eq!(std::fs::metadata(&path).unwrap().len(), 0);
        drop(s);
        std::fs::write(&path, [0u8; 100]).unwrap();
        assert!(FileStorage::open(&path).is_err(), "페이지 배수가 아닌 파일");
    }
}

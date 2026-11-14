//! 버퍼풀 — 고정 프레임에 페이지를 캐시한다. 핀 가드(RAII)·dirty write-back·LRU 내보내기(P2).
//!
//! `&self` API: 프레임 내용은 프레임마다 `RefCell`, 장부(페이지 테이블·핀·dirty·LRU 시각·통계)는 `RefCell` 하나.
//! 단일 스레드 전용. 같은 페이지를 읽기 가드 여럿 또는 쓰기 가드 하나로만 빌릴 수 있다.

use std::cell::{Ref, RefCell, RefMut};
use std::collections::HashMap;
use std::fmt;
use std::io;
use std::ops::{Deref, DerefMut};

use bytemuck::Zeroable;

use super::pager::Pager;
use super::{Page, PageId, Storage};

#[derive(Debug)]
pub enum PoolError {
    Io(io::Error),
    /// 모든 프레임이 핀 상태라 내보낼 프레임이 없다.
    AllPinned,
    /// 이 페이지에 이미 충돌하는 가드가 살아 있다(읽기 중 쓰기, 쓰기 중 읽기·쓰기).
    Borrowed(PageId),
}

impl From<io::Error> for PoolError {
    fn from(e: io::Error) -> Self {
        PoolError::Io(e)
    }
}

impl fmt::Display for PoolError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            PoolError::Io(e) => write!(f, "io: {e}"),
            PoolError::AllPinned => write!(f, "all frames are pinned"),
            PoolError::Borrowed(id) => write!(f, "page {id} is already borrowed"),
        }
    }
}

impl std::error::Error for PoolError {}

pub type Result<T> = std::result::Result<T, PoolError>;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PoolStats {
    /// 이미 프레임에 있던 페이지를 찾은 횟수 — 디스크 읽기 0.
    pub hits: u64,
    /// 디스크에서 읽어 와야 했던 횟수.
    pub misses: u64,
    /// 페이지가 든 프레임을 다른 페이지에 내준 횟수.
    pub evictions: u64,
}

#[derive(Clone, Copy, Default)]
struct FrameMeta {
    page: Option<PageId>,
    pins: u32,
    dirty: bool,
    /// 마지막으로 핀한 시각(장부 tick). 가장 작은 것이 LRU 희생자.
    last_used: u64,
}

struct Ledger {
    table: HashMap<PageId, usize>,
    meta: Vec<FrameMeta>,
    tick: u64,
    stats: PoolStats,
}

impl Ledger {
    fn unpin(&mut self, frame: usize, dirtied: bool) {
        let m = &mut self.meta[frame];
        debug_assert!(m.pins > 0);
        m.pins -= 1;
        m.dirty |= dirtied;
    }
}

pub struct BufferPool<S: Storage> {
    pager: RefCell<Pager<S>>,
    frames: Vec<RefCell<Box<Page>>>,
    ledger: RefCell<Ledger>,
}

impl<S: Storage> BufferPool<S> {
    pub fn new(pager: Pager<S>, capacity: usize) -> Self {
        assert!(capacity > 0, "buffer pool needs at least one frame");
        Self {
            pager: RefCell::new(pager),
            frames: (0..capacity)
                .map(|_| RefCell::new(Box::new(Page::zeroed())))
                .collect(),
            ledger: RefCell::new(Ledger {
                table: HashMap::new(),
                meta: vec![FrameMeta::default(); capacity],
                tick: 0,
                stats: PoolStats::default(),
            }),
        }
    }

    /// 읽기 가드. drop되면 언핀.
    pub fn fetch(&self, id: PageId) -> Result<PageRef<'_>> {
        let frame = self.pin(id)?;
        match self.frames[frame].try_borrow() {
            Ok(page) => Ok(PageRef {
                ledger: &self.ledger,
                frame,
                page,
            }),
            Err(_) => {
                self.ledger.borrow_mut().unpin(frame, false);
                Err(PoolError::Borrowed(id))
            }
        }
    }

    /// 쓰기 가드. 실제로 `&mut`로 건드렸을 때만 dirty가 된다. drop되면 언핀.
    pub fn fetch_mut(&self, id: PageId) -> Result<PageMut<'_>> {
        let frame = self.pin(id)?;
        match self.frames[frame].try_borrow_mut() {
            Ok(page) => Ok(PageMut {
                ledger: &self.ledger,
                frame,
                page,
                dirtied: false,
            }),
            Err(_) => {
                self.ledger.borrow_mut().unpin(frame, false);
                Err(PoolError::Borrowed(id))
            }
        }
    }

    /// 새 페이지 번호를 받고 0으로 채운 프레임을 쓰기 가드로 준다. 디스크 읽기 없음, 아직 디스크에 없으니 dirty.
    pub fn new_page(&self) -> Result<(PageId, PageMut<'_>)> {
        let mut ledger = self.ledger.borrow_mut();
        let frame = self.take_frame(&mut ledger)?;
        let id = self.pager.borrow_mut().allocate();
        let mut page = self.frames[frame].borrow_mut(); // 핀 0이던 프레임 — 살아 있는 가드 없음
        **page = Page::zeroed();
        ledger.tick += 1;
        ledger.meta[frame] = FrameMeta {
            page: Some(id),
            pins: 1,
            dirty: true,
            last_used: ledger.tick,
        };
        ledger.table.insert(id, frame);
        Ok((
            id,
            PageMut {
                ledger: &self.ledger,
                frame,
                page,
                dirtied: true,
            },
        ))
    }

    /// dirty 프레임을 전부 디스크에 쓰고 clean으로 만든다. 쓰기 가드가 살아 있는 페이지가 있으면 `Borrowed`.
    pub fn flush(&self) -> Result<()> {
        let mut ledger = self.ledger.borrow_mut();
        for frame in 0..self.frames.len() {
            let m = ledger.meta[frame];
            if let (Some(id), true) = (m.page, m.dirty) {
                let page = self.frames[frame]
                    .try_borrow()
                    .map_err(|_| PoolError::Borrowed(id))?;
                self.pager.borrow_mut().write(id, &page)?;
                ledger.meta[frame].dirty = false;
            }
        }
        Ok(())
    }

    pub fn stats(&self) -> PoolStats {
        self.ledger.borrow().stats
    }

    /// Pager의 실제 디스크 (읽기, 쓰기) 횟수.
    pub fn io_counts(&self) -> (u64, u64) {
        let p = self.pager.borrow();
        (p.reads(), p.writes())
    }

    pub fn capacity(&self) -> usize {
        self.frames.len()
    }

    pub fn is_resident(&self, id: PageId) -> bool {
        self.ledger.borrow().table.contains_key(&id)
    }

    pub fn is_dirty(&self, id: PageId) -> bool {
        let l = self.ledger.borrow();
        l.table.get(&id).is_some_and(|&f| l.meta[f].dirty)
    }

    /// 핀이 1 이상인 프레임 수.
    pub fn pinned_frames(&self) -> usize {
        self.ledger
            .borrow()
            .meta
            .iter()
            .filter(|m| m.pins > 0)
            .count()
    }

    /// 페이지를 프레임에 올리고 핀 1을 더한다. 프레임 번호를 돌려준다.
    fn pin(&self, id: PageId) -> Result<usize> {
        let mut ledger = self.ledger.borrow_mut();
        ledger.tick += 1;
        let tick = ledger.tick;
        if let Some(&frame) = ledger.table.get(&id) {
            let m = &mut ledger.meta[frame];
            m.pins += 1;
            m.last_used = tick;
            ledger.stats.hits += 1;
            return Ok(frame);
        }
        let frame = self.take_frame(&mut ledger)?;
        self.pager
            .borrow_mut()
            .read(id, &mut self.frames[frame].borrow_mut())?;
        ledger.stats.misses += 1;
        ledger.meta[frame] = FrameMeta {
            page: Some(id),
            pins: 1,
            dirty: false,
            last_used: tick,
        };
        ledger.table.insert(id, frame);
        Ok(frame)
    }

    /// 빈 프레임, 없으면 핀 0 중 가장 오래 안 쓴 프레임을 비워서 준다. dirty면 먼저 디스크에 쓴다.
    fn take_frame(&self, ledger: &mut Ledger) -> Result<usize> {
        if let Some(free) = ledger.meta.iter().position(|m| m.page.is_none()) {
            return Ok(free);
        }
        let victim = (0..ledger.meta.len())
            .filter(|&f| ledger.meta[f].pins == 0)
            .min_by_key(|&f| ledger.meta[f].last_used)
            .ok_or(PoolError::AllPinned)?;
        let old = ledger.meta[victim];
        let old_id = old.page.expect("non-free frame holds a page");
        if old.dirty {
            // 쓰기가 실패하면 장부를 건드리지 않고 돌아간다 — 페이지는 dirty인 채 프레임에 남는다.
            self.pager
                .borrow_mut()
                .write(old_id, &self.frames[victim].borrow())?;
        }
        ledger.table.remove(&old_id);
        ledger.meta[victim] = FrameMeta::default();
        ledger.stats.evictions += 1;
        Ok(victim)
    }
}

/// 읽기 가드 — 살아 있는 동안 페이지가 핀되어 내보내지지 않는다.
pub struct PageRef<'a> {
    ledger: &'a RefCell<Ledger>,
    frame: usize,
    page: Ref<'a, Box<Page>>,
}

impl Deref for PageRef<'_> {
    type Target = Page;
    fn deref(&self) -> &Page {
        &self.page
    }
}

impl Drop for PageRef<'_> {
    fn drop(&mut self) {
        self.ledger.borrow_mut().unpin(self.frame, false);
    }
}

/// 쓰기 가드 — `&mut Page`를 꺼낸 적이 있으면 drop 때 dirty로 표시하고 언핀한다.
pub struct PageMut<'a> {
    ledger: &'a RefCell<Ledger>,
    frame: usize,
    page: RefMut<'a, Box<Page>>,
    dirtied: bool,
}

impl Deref for PageMut<'_> {
    type Target = Page;
    fn deref(&self) -> &Page {
        &self.page
    }
}

impl DerefMut for PageMut<'_> {
    fn deref_mut(&mut self) -> &mut Page {
        self.dirtied = true;
        &mut self.page
    }
}

impl Drop for PageMut<'_> {
    fn drop(&mut self) {
        self.ledger.borrow_mut().unpin(self.frame, self.dirtied);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::MemStorage;

    fn pool(capacity: usize) -> BufferPool<MemStorage> {
        BufferPool::new(Pager::new(MemStorage::new()).unwrap(), capacity)
    }

    /// 새 페이지 n개를 만들어 첫 바이트에 번호를 적고 flush.
    fn seeded(capacity: usize, n: u64) -> BufferPool<MemStorage> {
        let p = pool(capacity);
        for _ in 0..n {
            let (id, mut g) = p.new_page().unwrap();
            g.0[0] = id as u8;
        }
        p.flush().unwrap();
        p
    }

    #[test]
    fn hit_needs_no_disk_read() {
        let p = seeded(2, 1);
        let (r0, _) = p.io_counts();
        assert_eq!(p.fetch(0).unwrap().0[0], 0);
        assert_eq!(p.fetch(0).unwrap().0[0], 0);
        assert_eq!(p.io_counts().0, r0);
        assert_eq!(
            p.stats(),
            PoolStats {
                hits: 2,
                misses: 0,
                evictions: 0
            }
        );
    }

    #[test]
    fn lru_evicts_least_recently_used_and_writes_back_only_dirty() {
        let p = seeded(2, 3); // 페이지 0·1·2 — 프레임 2개라 0은 이미 밀려남
        let (_, w0) = p.io_counts();
        p.fetch(1).unwrap();
        p.fetch_mut(2).unwrap().0[1] = 42; // 2 dirty, 최근 사용
        p.fetch(0).unwrap(); // miss → 핀 0 중 가장 오래된 1(clean) 내보냄 — 쓰기 없음
        assert!(!p.is_resident(1) && p.is_resident(2) && p.is_dirty(2));
        assert_eq!(p.io_counts().1, w0);
        p.fetch(1).unwrap(); // miss → 이번 희생자는 2(dirty) → 먼저 디스크에 씀
        assert!(!p.is_resident(2));
        assert_eq!(p.io_counts().1, w0 + 1);
        assert_eq!(
            p.fetch(2).unwrap().0[1],
            42,
            "내보낸 변경이 디스크에서 돌아온다"
        );
    }

    #[test]
    fn pinned_page_is_never_evicted_and_all_pinned_is_error() {
        let p = seeded(2, 3);
        let a = p.fetch(1).unwrap();
        let b = p.fetch(2).unwrap();
        assert!(matches!(p.fetch(0), Err(PoolError::AllPinned)));
        assert!(p.is_resident(1) && p.is_resident(2));
        drop(a);
        assert_eq!(p.fetch(0).unwrap().0[0], 0); // 1이 언핀되어 희생자가 됨
        assert!(!p.is_resident(1) && p.is_resident(2));
        drop(b);
        assert_eq!(p.pinned_frames(), 0);
    }

    #[test]
    fn conflicting_guards_are_rejected_without_leaking_pins() {
        let p = seeded(2, 1);
        let r = p.fetch(0).unwrap();
        assert!(matches!(p.fetch_mut(0), Err(PoolError::Borrowed(0))));
        let r2 = p.fetch(0).unwrap(); // 읽기끼리는 함께
        drop((r, r2));
        assert_eq!(p.pinned_frames(), 0);
        let w = p.fetch_mut(0).unwrap();
        assert!(matches!(p.fetch(0), Err(PoolError::Borrowed(0))));
        assert!(
            matches!(p.flush(), Ok(())),
            "clean 쓰기 가드는 flush 대상 아님"
        );
        drop(w);
    }

    #[test]
    fn read_only_write_guard_stays_clean_and_flush_clears_dirty() {
        let p = seeded(2, 2);
        let (_, w0) = p.io_counts();
        let _ = p.fetch_mut(0).unwrap().0[0]; // 읽기만
        assert!(!p.is_dirty(0));
        p.fetch_mut(1).unwrap().0[0] = 9;
        p.flush().unwrap();
        assert_eq!(p.io_counts().1, w0 + 1, "dirty 1장만 쓴다");
        assert!(!p.is_dirty(1));
        p.flush().unwrap();
        assert_eq!(p.io_counts().1, w0 + 1, "두 번째 flush는 쓸 것 없음");
    }
}

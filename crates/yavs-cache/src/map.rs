//! Reading an entry file: mapped into memory when the system allows it, else read into a buffer.
//!
//! The only `unsafe` in the crate (the workspace forbids it; this crate denies it and allows it
//! here), for the call that maps a file and the calls that return free heap to the system or ask
//! for huge pages on it.

use std::fs;
use std::io::Read as _;
use std::path::Path;

/// The bytes of an entry file.
pub(crate) enum Data {
    Mapped(memmap2::Mmap),
    Read(Vec<u8>),
}

impl std::fmt::Debug for Data {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{} bytes", self.len())
    }
}

impl std::ops::Deref for Data {
    type Target = [u8];

    fn deref(&self) -> &[u8] {
        match self {
            Data::Mapped(m) => m,
            Data::Read(v) => v,
        }
    }
}

/// The file's bytes, or `None` when it cannot be opened. A file that cannot be mapped (an empty
/// one, a file system without mapping) is read instead. With `touch` (Unix) the entry's time is
/// refreshed through the open file (the old way opened it again for writing: three system calls);
/// failing is harmless.
pub(crate) fn open(path: &Path, touch: bool) -> Option<Data> {
    let mut f = fs::File::open(path).ok()?;
    // One question to the system serves the length of the mapping.
    let meta = f.metadata().ok()?;
    #[cfg(unix)]
    if touch {
        let _ = f.set_modified(std::time::SystemTime::now());
    }
    #[cfg(not(unix))]
    let _ = touch;
    // SAFETY: entries are never written in place: a writer fills a temporary file and renames it
    // over the entry, a prune or clean removes the name, and an open mapping keeps its pages
    // either way (Unix; on Windows removing a mapped file fails and is ignored). The bytes a
    // mapping shows therefore do not change while it lives; only something outside yavs that
    // truncates an entry in place could invalidate them (a signal on access, the same
    // exposure any mapped file has). `Entry::get` verifies the length and the checksum of what
    // it maps before anything is decoded from it.
    if let Ok(m) = unsafe {
        memmap2::MmapOptions::new()
            .len(usize::try_from(meta.len()).ok()?)
            .populate()
            .map(&f)
    } {
        return Some(Data::Mapped(m));
    }
    let mut v = Vec::new();
    f.read_to_end(&mut v).ok()?;
    Some(Data::Read(v))
}

/// Give the memory the allocator holds but the program no longer uses back to the system.
#[cfg(all(target_os = "linux", target_env = "gnu"))]
pub(crate) fn trim_heap() {
    unsafe extern "C" {
        fn malloc_trim(pad: usize) -> i32;
    }
    // SAFETY: `malloc_trim` takes no pointers and only returns free pages to the system.
    unsafe {
        malloc_trim(0);
    }
}

#[cfg(not(all(target_os = "linux", target_env = "gnu")))]
pub(crate) fn trim_heap() {}

/// Ask for transparent huge pages on the heap, in this process, once at start (#556). The
/// allocator takes memory from the system in small steps, so a run that builds a few tens of MiB
/// takes thousands of page faults (a fifth of a cold elaboration). The heap grows in one big step
/// instead, and that step is marked `MADV_HUGEPAGE`, which needs a system with transparent huge
/// pages in `madvise` mode; anything else leaves the allocator as it was. `YAVS_HUGE_HEAP=0`
/// switches it off.
#[cfg(all(target_os = "linux", target_env = "gnu"))]
pub(crate) fn huge_heap() {
    use std::ffi::{c_int, c_void};
    unsafe extern "C" {
        fn mallopt(param: c_int, value: c_int) -> c_int;
        fn sbrk(increment: isize) -> *mut c_void;
        fn madvise(addr: *mut c_void, len: usize, advice: c_int) -> c_int;
    }
    const M_TRIM_THRESHOLD: c_int = -1;
    const M_TOP_PAD: c_int = -2;
    const M_MMAP_THRESHOLD: c_int = -3;
    const MADV_HUGEPAGE: c_int = 14;
    const HUGE: usize = 2 << 20;
    // The first growth step: 64 MiB covers a typical elaboration; measured larger steps only helped a CPU-core simulation.
    const PAD: usize = 64 << 20;
    if std::env::var_os("YAVS_HUGE_HEAP").is_some_and(|v| v == "0") {
        return;
    }
    // One read into a small buffer ("always [madvise] never" is 23 bytes): `read_to_string`
    // also asks for the size and reads again to find the end.
    let mut mode = [0u8; 64];
    let n = std::fs::File::open("/sys/kernel/mm/transparent_hugepage/enabled")
        .and_then(|mut f| std::io::Read::read(&mut f, &mut mode))
        .unwrap_or(0);
    if !mode[..n].windows(9).any(|w| w == b"[madvise]") {
        return;
    }
    // SAFETY: `mallopt`, `sbrk(0)` and `madvise` on the heap's own range take no pointers
    // the program owns; a failed call changes nothing.
    unsafe {
        // Setting any of these turns off the allocator's own tuning of the mapping threshold,
        // so it is set to its upper limit (the value the tuning grows to anyway).
        mallopt(M_MMAP_THRESHOLD, 32 << 20);
        mallopt(M_TOP_PAD, PAD as c_int);
        mallopt(M_TRIM_THRESHOLD, (2 * PAD) as c_int);
        let from = sbrk(0) as usize;
        // Make the allocator extend the heap now; the block is freed into the new top.
        let grow: Vec<u8> = Vec::with_capacity(1 << 20);
        std::hint::black_box(&grow);
        drop(grow);
        let to = sbrk(0) as usize;
        if let Some(start) = from.checked_next_multiple_of(HUGE)
            && to > start
        {
            madvise(start as *mut c_void, to - start, MADV_HUGEPAGE);
        }
    }
}

#[cfg(not(all(target_os = "linux", target_env = "gnu")))]
pub(crate) fn huge_heap() {}

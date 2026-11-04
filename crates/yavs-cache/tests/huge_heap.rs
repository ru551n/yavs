//! `huge_heap` marks the break heap for transparent huge pages (#556) unless `YAVS_HUGE_HEAP=0`,
//! and keeps marking it as the allocator extends it.
//! No harness: the check runs in a child process on its main thread.

use std::process::Command;

fn thp_madvise() -> bool {
    std::fs::read_to_string("/sys/kernel/mm/transparent_hugepage/enabled")
        .is_ok_and(|s| s.contains("[madvise]"))
}

/// Bytes of the break heap's mappings that do not carry the huge page flag (`hg`).
fn heap_unmarked() -> usize {
    let smaps = std::fs::read_to_string("/proc/self/smaps").unwrap();
    let (mut heap, mut size, mut total) = (false, 0, 0);
    for l in smaps.lines() {
        if l.contains(" [heap]") {
            heap = true;
            let (a, b) = l
                .split_whitespace()
                .next()
                .unwrap()
                .split_once('-')
                .unwrap();
            size = usize::from_str_radix(b, 16).unwrap() - usize::from_str_radix(a, 16).unwrap();
        } else if l.starts_with("VmFlags:") {
            if heap && !l.split_whitespace().any(|f| f == "hg") {
                total += size;
            }
            heap = false;
        }
    }
    total
}

/// Wait up to 3 s for all but a few MiB of the break heap to carry the flag.
fn all_marked(when: &str) {
    let mut small = heap_unmarked();
    for _ in 0..60 {
        if small <= 4 << 20 {
            return;
        }
        std::thread::sleep(std::time::Duration::from_millis(50));
        small = heap_unmarked();
    }
    panic!("{small} bytes of the heap without huge pages {when}");
}

/// Whether a mapping of the break heap carries the huge page flag (`hg`).
fn heap_marked() -> bool {
    let smaps = std::fs::read_to_string("/proc/self/smaps").unwrap();
    let mut heap = false;
    for l in smaps.lines() {
        if l.contains(" [heap]") {
            heap = true;
        } else if l.starts_with("VmFlags:") {
            if heap && l.split_whitespace().any(|f| f == "hg") {
                return true;
            }
            heap = false;
        }
    }
    false
}

fn main() {
    if let Some(want) = std::env::args().nth(1) {
        yavs_cache::huge_heap();
        let v = vec![1u8; 8 << 20];
        assert_eq!(heap_marked(), want == "on", "{} bytes", v.len());
        if want == "on" {
            // Well past the first step: each later step is a new mapping, marked by the thread
            // that watches the heap, which is given time to come round.
            let more: Vec<Vec<u8>> = (0..200).map(|_| vec![1u8; 1 << 20]).collect();
            all_marked("past the first step");
            // The heap given back and grown again is another new mapping.
            drop(more);
            yavs_cache::trim_heap();
            let again: Vec<Vec<u8>> = (0..200).map(|_| vec![1u8; 1 << 20]).collect();
            all_marked("after the heap shrank");
            drop(again);
        }
        return;
    }
    if !cfg!(all(target_os = "linux", target_env = "gnu")) || !thp_madvise() {
        println!("skipped: needs Linux, glibc and transparent huge pages in madvise mode");
        return;
    }
    // The environment of each child: yavs's own switch, and the user's allocator tuning.
    let cases = [
        ("on", "YAVS_HUGE_HEAP", "1"),
        ("off", "YAVS_HUGE_HEAP", "0"),
        ("off", "GLIBC_TUNABLES", "glibc.malloc.top_pad=1048576"),
        (
            "off",
            "GLIBC_TUNABLES",
            "glibc.malloc.hugetlb=1:glibc.malloc.arena_max=4",
        ),
        ("off", "MALLOC_TRIM_THRESHOLD_", "1048576"),
        ("on", "GLIBC_TUNABLES", "glibc.malloc.arena_max=4"),
        ("on", "GLIBC_TUNABLES", "glibc.pthread.rseq=0"),
    ];
    for (want, var, value) in cases {
        let st = Command::new(std::env::current_exe().unwrap())
            .arg(want)
            .env(var, value)
            .status()
            .unwrap();
        assert!(st.success(), "{want} with {var}={value}");
    }
}

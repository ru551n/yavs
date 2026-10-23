//! `huge_heap` marks the break heap for transparent huge pages (#556) unless `YAVS_HUGE_HEAP=0`.
//! No harness: the check runs in a child process on its main thread.

use std::process::Command;

fn thp_madvise() -> bool {
    std::fs::read_to_string("/sys/kernel/mm/transparent_hugepage/enabled")
        .is_ok_and(|s| s.contains("[madvise]"))
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
        return;
    }
    if !cfg!(all(target_os = "linux", target_env = "gnu")) || !thp_madvise() {
        println!("skipped: needs Linux, glibc and transparent huge pages in madvise mode");
        return;
    }
    for (want, off) in [("on", "1"), ("off", "0")] {
        let st = Command::new(std::env::current_exe().unwrap())
            .arg(want)
            .env("YAVS_HUGE_HEAP", off)
            .status()
            .unwrap();
        assert!(st.success(), "{want}");
    }
}

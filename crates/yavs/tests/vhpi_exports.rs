//! The `yavs` executable must export every VHPI symbol of the vendored
//! `vhpi_user.h`, or no plugin can bind to it (#266). Reads the ELF
//! `.dynsym` of the built binary, so Linux only.
#![cfg(all(target_os = "linux", target_pointer_width = "64"))]

use std::collections::BTreeSet;

const HEADER: &str = include_str!("../../yavs-vhpi/include/vhpi_user.h");

/// Function names (`XXTERN type vhpi_x (`) and data names (`PLI_VEXTERN ... vhpiX;`).
fn header_symbols() -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    for line in HEADER.lines() {
        if line.starts_with("XXTERN") {
            let name = line.split('(').next().unwrap().split_whitespace().last();
            out.insert(name.unwrap().to_owned());
        } else if line.starts_with("PLI_VEXTERN") {
            out.insert(
                line.trim_end_matches(';')
                    .split_whitespace()
                    .last()
                    .unwrap()
                    .to_owned(),
            );
        }
    }
    out
}

fn u(b: &[u8], at: usize, n: usize) -> usize {
    let mut a = [0u8; 8];
    a[..n].copy_from_slice(&b[at..at + n]);
    u64::from_le_bytes(a) as usize
}

/// Names of the defined symbols in `.dynsym` of a little-endian ELF64 file.
fn dynsym(b: &[u8]) -> BTreeSet<String> {
    assert_eq!(&b[..4], b"\x7fELF");
    assert_eq!((b[4], b[5]), (2, 1), "expected ELF64 little endian");
    let (shoff, shentsize, shnum) = (u(b, 0x28, 8), u(b, 0x3a, 2), u(b, 0x3c, 2));
    let sh = |i: usize| shoff + i * shentsize;
    let mut out = BTreeSet::new();
    for i in 0..shnum {
        if u(b, sh(i) + 4, 4) != 11 {
            continue; // not SHT_DYNSYM
        }
        let (off, size, strtab) = (
            u(b, sh(i) + 0x18, 8),
            u(b, sh(i) + 0x20, 8),
            u(b, sh(i) + 0x28, 4),
        );
        let stroff = u(b, sh(strtab) + 0x18, 8);
        for s in (off..off + size).step_by(24) {
            if u(b, s + 6, 2) == 0 {
                continue; // SHN_UNDEF: imported, not defined
            }
            let n = stroff + u(b, s, 4);
            let end = n + b[n..].iter().position(|&c| c == 0).unwrap();
            out.insert(String::from_utf8_lossy(&b[n..end]).into_owned());
        }
    }
    out
}

#[test]
fn yavs_exports_every_header_symbol() {
    let want = header_symbols();
    assert!(want.len() > 30, "header parse found only {want:?}");
    let have = dynsym(&std::fs::read(env!("CARGO_BIN_EXE_yavs")).unwrap());
    let missing: Vec<_> = want.difference(&have).collect();
    assert!(missing.is_empty(), "yavs does not export: {missing:?}");
}

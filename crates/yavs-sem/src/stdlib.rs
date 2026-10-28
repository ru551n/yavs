//! The std/ieee sources built into the binary (`build.rs`). They live under the virtual
//! root [`EMBEDDED`]; a path below it is read from the table, every other path from disk.

use std::path::{Component, Path};

include!(concat!(env!("OUT_DIR"), "/stdlib_files.rs"));

/// The root of the built-in tree. Diagnostics show `<yavs>/lib/ieee/2008/numeric_std.vhdl`.
pub const EMBEDDED: &str = "<yavs>/lib";

/// `path` for a diagnostic: the virtual built-in tree always shows `/`, as on Linux.
pub(crate) fn display(path: &Path) -> String {
    let s = path.display().to_string();
    if path.starts_with(EMBEDDED) {
        s.replace('\\', "/")
    } else {
        s
    }
}

fn embedded(path: &Path) -> Option<&'static [u8]> {
    let rel = path.strip_prefix(EMBEDDED).ok()?;
    let key: Vec<_> = rel
        .components()
        .map(|c| match c {
            Component::Normal(s) => s.to_str(),
            _ => None,
        })
        .collect::<Option<_>>()?;
    let key = key.join("/");
    FILES.iter().find(|(k, _)| *k == key).map(|&(_, b)| b)
}

pub(crate) fn is_file(path: &Path) -> bool {
    embedded(path).is_some() || path.is_file()
}

/// The primary units of the built-in directory `dir` (`<yavs>/lib/ieee/2008`), by file name.
// ponytail: built-in tree only; a library on disk is suggested from its manifest instead.
pub(crate) fn unit_stems(dir: &Path) -> Vec<String> {
    let Ok(rel) = dir.strip_prefix(EMBEDDED) else {
        return Vec::new();
    };
    let parts: Vec<&str> = rel
        .components()
        .filter_map(|c| match c {
            Component::Normal(s) => s.to_str(),
            _ => None,
        })
        .collect();
    let prefix = format!("{}/", parts.join("/"));
    FILES
        .iter()
        .filter_map(|(k, _)| k.strip_prefix(&prefix)?.strip_suffix(".vhdl"))
        .filter(|s| !s.ends_with("-body") && !s.contains('/'))
        .map(str::to_lowercase)
        .collect()
}

pub(crate) fn read(path: &Path) -> std::io::Result<Vec<u8>> {
    match embedded(path) {
        Some(b) => Ok(b.to_vec()),
        None => std::fs::read(path),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn display_uses_forward_slashes() {
        let p = Path::new(EMBEDDED).join("ieee").join("2008");
        assert_eq!(display(&p), "<yavs>/lib/ieee/2008");
    }

    #[test]
    fn embedded_tree_is_complete() {
        assert!(is_file(
            &Path::new(EMBEDDED).join("ieee/2008/numeric_std.vhdl")
        ));
        assert!(is_file(
            &Path::new(EMBEDDED)
                .join("std")
                .join("2019")
                .join("standard.vhdl")
        ));
        assert!(!is_file(&Path::new(EMBEDDED).join("ieee/2008/nope.vhdl")));
        assert_eq!(FILES.len(), 111);
    }
}

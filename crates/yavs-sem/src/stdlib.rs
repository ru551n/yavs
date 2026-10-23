//! The std/ieee sources built into the binary (`build.rs`). They live under the virtual
//! root [`EMBEDDED`]; a path below it is read from the table, every other path from disk.

use std::path::{Component, Path};

include!(concat!(env!("OUT_DIR"), "/stdlib_files.rs"));

/// The root of the built-in tree. Diagnostics show `<yavs>/lib/ieee/2008/numeric_std.vhdl`.
pub const EMBEDDED: &str = "<yavs>/lib";

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

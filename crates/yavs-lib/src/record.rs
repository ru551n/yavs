//! The record of a source file: the units it defines with their original names and the units they
//! depend on. The index (`manifest.rs`) knows where units are; only `closure` (dependencies) and
//! the analysed-unit cache (a unit as the library recorded it) read records.
//!
//! A record is never rewritten: it is named by the digest of its text (`RECORD_DIR/<digest>.toml`)
//! and the index names the digest. A reader holding the library lock therefore reads the
//! record its index means, whatever an interrupted write left behind, and a record that was
//! damaged is found by its digest.

use crate::manifest::{UnitKind, UnitRecord, UnitRef};
use crate::text::{Cur, put_line, put_str};
use crate::{Result, error::err, error::io};
use sha2::{Digest, Sha256};
use std::path::Path;

/// The text of the record of `path` defining `units`.
pub(crate) fn render(path: &str, units: &[UnitRecord]) -> String {
    let mut o = String::with_capacity(units.len() * 160 + path.len() + 32);
    put_line(&mut o, "path", path);
    for u in units {
        o.push_str("\n[[unit]]\n");
        put_line(&mut o, "kind", u.kind.text());
        put_line(&mut o, "name", &u.name);
        put_line(&mut o, "key", &u.key);
        if let Some(p) = &u.primary {
            put_line(&mut o, "primary", p);
        }
        put_line(&mut o, "std", &u.std);
        o.push_str("depends_on = [");
        for (i, d) in u.depends_on.iter().enumerate() {
            if i > 0 {
                o.push_str(", ");
            }
            o.push('[');
            put_str(&mut o, &d.library);
            o.push_str(", ");
            put_str(&mut o, d.kind.text());
            o.push_str(", ");
            put_str(&mut o, &d.key);
            o.push(']');
        }
        o.push_str("]\n");
    }
    o
}

/// The digest that names a record: the first 16 bytes of the SHA-256 of its text.
pub(crate) fn digest(text: &str) -> u128 {
    let d = Sha256::digest(text.as_bytes());
    u128::from_be_bytes(d[..16].try_into().expect("16 bytes"))
}

pub(crate) fn file_name(digest: u128) -> String {
    format!("{digest:032x}.toml")
}

/// The units of the record `text` of the file `path`, or why it is not one.
fn parse(text: &str, path: &Path) -> std::result::Result<Vec<UnitRecord>, String> {
    let bad = |n: usize, why: &str| format!("line {}: {why}", n + 1);
    let mut lines = text.lines().enumerate();
    let (n, first) = lines.next().ok_or("empty")?;
    let mut p = String::new();
    let mut c = Cur::new(
        first
            .strip_prefix("path = ")
            .ok_or_else(|| bad(n, "expected `path`"))?,
    );
    c.string(&mut p)
        .filter(|()| c.done())
        .ok_or_else(|| bad(n, "bad path"))?;
    if Path::new(&p) != path {
        return Err(format!("it is the record of `{p}`"));
    }
    let mut units: Vec<UnitRecord> = Vec::new();
    // Fields of the unit being read.
    let (mut kind, mut name, mut key, mut primary, mut std, mut deps) =
        (None, None, None, None, None, None::<Vec<UnitRef>>);
    let mut in_unit = false;
    macro_rules! end_unit {
        ($n:expr) => {
            if std::mem::take(&mut in_unit) {
                units.push(UnitRecord {
                    kind: kind.take().ok_or_else(|| bad($n, "a unit lacks `kind`"))?,
                    name: name.take().ok_or_else(|| bad($n, "a unit lacks `name`"))?,
                    key: key.take().ok_or_else(|| bad($n, "a unit lacks `key`"))?,
                    primary: primary.take(),
                    std: std.take().ok_or_else(|| bad($n, "a unit lacks `std`"))?,
                    depends_on: deps
                        .take()
                        .ok_or_else(|| bad($n, "a unit lacks `depends_on`"))?,
                    file: path.to_path_buf(),
                });
            }
        };
    }
    let mut last = 0;
    for (n, line) in lines {
        last = n;
        if line.is_empty() {
            continue;
        }
        if line == "[[unit]]" {
            end_unit!(n);
            in_unit = true;
            continue;
        }
        let sp = line
            .bytes()
            .position(|b| b == b' ')
            .ok_or_else(|| bad(n, "expected `key = value`"))?;
        let val = line[sp..]
            .strip_prefix(" = ")
            .ok_or_else(|| bad(n, "expected `key = value`"))?;
        if !in_unit {
            return Err(bad(n, "a line outside a unit"));
        }
        let mut c = Cur::new(val);
        let string = |c: &mut Cur| {
            let mut s = String::new();
            c.string(&mut s)
                .filter(|()| c.done())
                .map(|()| s)
                .ok_or_else(|| bad(n, "bad string"))
        };
        match &line[..sp] {
            "kind" => {
                let k = string(&mut c)?;
                kind = Some(UnitKind::from_text(&k).ok_or_else(|| bad(n, "unknown unit kind"))?);
            }
            "name" => name = Some(string(&mut c)?),
            "key" => key = Some(string(&mut c)?),
            "primary" => primary = Some(string(&mut c)?),
            "std" => std = Some(string(&mut c)?),
            "depends_on" => {
                let mut v = Vec::new();
                let sep = |c: &mut Cur| c.eat(b',').and_then(|()| c.eat(b' '));
                c.eat(b'[').ok_or_else(|| bad(n, "expected ["))?;
                while c.peek() != Some(b']') {
                    let (mut l, mut k, mut key) = (String::new(), String::new(), String::new());
                    let ok = c.eat(b'[').is_some()
                        && c.string(&mut l).is_some()
                        && sep(&mut c).is_some()
                        && c.string(&mut k).is_some()
                        && sep(&mut c).is_some()
                        && c.string(&mut key).is_some()
                        && c.eat(b']').is_some()
                        && (c.peek() == Some(b']') || sep(&mut c).is_some());
                    let kind = UnitKind::from_text(&k).filter(|_| ok);
                    v.push(UnitRef {
                        library: l,
                        kind: kind.ok_or_else(|| bad(n, "bad dependency"))?,
                        key,
                    });
                }
                c.eat(b']')
                    .filter(|()| c.done())
                    .ok_or_else(|| bad(n, "bad dependencies"))?;
                deps = Some(v);
            }
            _ => return Err(bad(n, "unknown key")),
        }
    }
    end_unit!(last);
    Ok(units)
}

/// The record `digest` in `dir` of the file `path`, checked against its digest.
pub(crate) fn load(dir: &Path, digest_: u128, path: &Path) -> Result<Vec<UnitRecord>> {
    let rp = dir.join(file_name(digest_));
    let gone = |what: &str| {
        Box::new(
            err("L0003", format!("library record `{}` of `{}` {what}", rp.display(), path.display()))
                .with_help(format!(
                    "the library was changed outside yavs: delete the library directory `{}` and analyze its files again",
                    dir.parent().unwrap_or(dir).display()
                )),
        )
    };
    let text = match std::fs::read_to_string(&rp) {
        Ok(t) => t,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Err(gone("is missing")),
        Err(e) => return Err(io("L0002", "read", &rp, &e)),
    };
    if digest(&text) != digest_ {
        return Err(gone("is damaged (its digest differs)"));
    }
    parse(&text, path).map_err(|why| Box::new((*gone("is corrupt")).with_note(why)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::manifest::fold;

    fn units() -> Vec<UnitRecord> {
        vec![
            UnitRecord::new(UnitKind::Package, "Pkg_A", "2008"),
            UnitRecord::package_body("Pkg_A", "2008").depends("work", UnitKind::Package, "pkg_a"),
            UnitRecord::architecture("Ent", "Rtl", "2019")
                .depends("ieee", UnitKind::Package, "std_logic_1164")
                .depends("lib", UnitKind::Entity, "\\x \"y\"\\"),
            UnitRecord::new(UnitKind::Context, "ctx", "2019"),
        ]
    }

    #[test]
    fn records_round_trip() {
        for path in ["/x/a.vhd", "/x/a\"b\\c\n.vhd"] {
            let mut u = units();
            for r in &mut u {
                r.file = path.into();
            }
            let text = render(path, &u);
            assert_eq!(parse(&text, Path::new(path)), Ok(u));
            // Another file's record is not this file's.
            assert!(
                parse(&text, Path::new("/x/other.vhd"))
                    .unwrap_err()
                    .contains("record of")
            );
        }
        assert_eq!(fold("A"), "a");
    }

    #[test]
    fn digest_names_the_text() {
        let t = render("/x/a.vhd", &units());
        let t2 = render("/x/b.vhd", &units());
        assert_ne!(digest(&t), digest(&t2));
        assert_eq!(digest(&t), digest(&t.clone()));
        assert_eq!(file_name(1).len(), 32 + 5);
    }

    #[test]
    fn bad_records() {
        let p = Path::new("/x/a.vhd");
        let t = render("/x/a.vhd", &units());
        for (from, to, why) in [
            (
                "kind = \"package\"",
                "kind = \"packag\"",
                "unknown unit kind",
            ),
            ("key = \"pkg_a\"\n", "", "lacks `key`"),
            ("depends_on = []", "depends_on = [", "bad"),
            (
                "depends_on = []",
                "depends_on = [[\"a\"]]",
                "bad dependency",
            ),
            ("std = \"2008\"", "stdd = \"2008\"", "unknown key"),
        ] {
            let e = parse(&t.replacen(from, to, 1), p).unwrap_err();
            assert!(e.contains(why), "{from}: {e}");
        }
        assert!(parse("", p).is_err());
        assert!(
            parse("kind = \"package\"\n", p)
                .unwrap_err()
                .contains("path")
        );
    }
}

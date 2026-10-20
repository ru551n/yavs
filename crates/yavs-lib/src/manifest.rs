use crate::{Result, error::err};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::path::PathBuf;

pub const MANIFEST_FILE: &str = "yavs-lib.toml";
pub const LOCK_FILE: &str = "_YAVS_LIB";
pub(crate) const FORMAT: u32 = 2;

/// Case-fold an identifier: basic identifiers are case-insensitive,
/// extended identifiers (`\Foo\`) are not.
pub fn fold(name: &str) -> String {
    if name.starts_with('\\') {
        name.to_string()
    } else {
        name.to_ascii_lowercase()
    }
}

/// `sha256:<hex>` of file contents.
pub fn hash_content(bytes: &[u8]) -> String {
    let d = Sha256::digest(bytes);
    let mut s = String::from("sha256:");
    for b in d.iter() {
        s.push_str(&format!("{b:02x}"));
    }
    s
}

/// Size and modification time (ns since the epoch) of a file, taken to skip re-hashing it later.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Stamp {
    pub size: u64,
    pub mtime_ns: i64,
}

/// How long a file must have been unmodified before its [`Stamp`] is trusted: a coarse file
/// system clock can give a later edit of the same size the same modification time.
const STAMP_AGE: std::time::Duration = std::time::Duration::from_secs(2);

impl Stamp {
    /// The stamp of `path` now; `None` when it cannot be read or was modified within the last
    /// two seconds (then the content hash is always checked).
    pub fn of(path: &std::path::Path) -> Option<Stamp> {
        Self::of_meta(&std::fs::metadata(path).ok()?)
    }

    pub(crate) fn of_meta(m: &std::fs::Metadata) -> Option<Stamp> {
        let t = m.modified().ok()?;
        if t.checked_add(STAMP_AGE)? > std::time::SystemTime::now() {
            return None;
        }
        let ns = t.duration_since(std::time::UNIX_EPOCH).ok()?.as_nanos();
        Some(Stamp {
            size: m.len(),
            mtime_ns: i64::try_from(ns).ok()?,
        })
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UnitKind {
    Entity,
    Architecture,
    Package,
    PackageBody,
    PackageInstance,
    Configuration,
    Context,
}

impl UnitKind {
    pub fn describe(self) -> &'static str {
        match self {
            UnitKind::Entity => "entity",
            UnitKind::Architecture => "architecture",
            UnitKind::Package => "package",
            UnitKind::PackageBody => "package body",
            UnitKind::PackageInstance => "package instance",
            UnitKind::Configuration => "configuration",
            UnitKind::Context => "context",
        }
    }
}

/// A reference to a unit in some library (library name folded).
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct UnitRef {
    pub library: String,
    pub kind: UnitKind,
    pub key: String,
}

impl UnitRef {
    pub fn new(library: &str, kind: UnitKind, name: &str) -> Self {
        UnitRef {
            library: fold(library),
            kind,
            key: fold(name),
        }
    }
}

/// One design unit defined by an analyzed file.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct UnitRecord {
    pub kind: UnitKind,
    /// Original spelling.
    pub name: String,
    /// Folded lookup key. Architectures use `entity/arch`.
    pub key: String,
    /// Folded key of the entity (architecture) or package (body) this unit belongs to.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub primary: Option<String>,
    pub std: String,
    #[serde(default)]
    pub depends_on: Vec<UnitRef>,
    /// Filled in by the library on load and update; not stored per unit.
    #[serde(skip)]
    pub file: PathBuf,
}

impl UnitRecord {
    pub fn new(kind: UnitKind, name: &str, std: &str) -> Self {
        UnitRecord {
            kind,
            name: name.into(),
            key: fold(name),
            primary: None,
            std: std.into(),
            depends_on: vec![],
            file: PathBuf::new(),
        }
    }
    /// Architecture `arch` of entity `entity`.
    pub fn architecture(entity: &str, arch: &str, std: &str) -> Self {
        UnitRecord {
            name: arch.into(),
            key: format!("{}/{}", fold(entity), fold(arch)),
            primary: Some(fold(entity)),
            ..Self::new(UnitKind::Architecture, arch, std)
        }
    }
    /// Body of package `pkg`.
    pub fn package_body(pkg: &str, std: &str) -> Self {
        UnitRecord {
            primary: Some(fold(pkg)),
            ..Self::new(UnitKind::PackageBody, pkg, std)
        }
    }
    pub fn depends(mut self, library: &str, kind: UnitKind, name: &str) -> Self {
        self.depends_on.push(UnitRef::new(library, kind, name));
        self
    }
    /// Name for messages: `kind name`.
    pub fn display(&self) -> String {
        match (self.kind, &self.primary) {
            (UnitKind::Architecture, Some(e)) => format!("architecture {} of {e}", self.name),
            _ => format!("{} {}", self.kind.describe(), self.name),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct FileEntry {
    /// Absolute path as analyzed.
    pub path: PathBuf,
    pub hash: String,
    /// VHDL standard this file was analyzed with (per file: projects may mix 2008 and 2019).
    pub std: String,
    /// Size of the file when `hash` was taken, if it was old enough to trust (see [`Stamp`]).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub size: Option<u64>,
    /// Modification time (ns since the epoch) of the file when `hash` was taken.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mtime_ns: Option<i64>,
    #[serde(default, rename = "unit")]
    pub units: Vec<UnitRecord>,
}

impl FileEntry {
    /// The file on disk is the one `hash` was taken of, by size and modification time alone.
    pub fn unchanged(&self) -> bool {
        match (self.size, self.mtime_ns) {
            (Some(size), Some(mtime_ns)) => Stamp::of(&self.path) == Some(Stamp { size, mtime_ns }),
            _ => false,
        }
    }
}

/// Contents of `yavs-lib.toml`. Files are in compile order.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Manifest {
    pub format: u32,
    pub name: String,
    #[serde(default, rename = "file")]
    pub files: Vec<FileEntry>,
}

impl Manifest {
    pub(crate) fn new(name: &str) -> Self {
        Manifest {
            format: FORMAT,
            name: name.into(),
            files: vec![],
        }
    }

    pub(crate) fn parse(text: &str, origin: &std::path::Path) -> Result<Self> {
        if let Some(m) = parse_written(text) {
            return Ok(m);
        }
        #[derive(Deserialize)]
        struct Head {
            format: u32,
        }
        if let Ok(h) = toml::from_str::<Head>(text)
            && h.format != FORMAT
        {
            return Err(Box::new(
                err(
                    "L0004",
                    format!("library manifest `{}` has format version {}, this yavs reads version {FORMAT}", origin.display(), h.format),
                )
                .with_help("re-analyze the library's files to rewrite the manifest"),
            ));
        }
        let mut m: Manifest = toml::from_str(text).map_err(|e| {
            Box::new(
                err(
                    "L0003",
                    format!("library manifest `{}` is corrupt", origin.display()),
                )
                .with_note(e.to_string()),
            )
        })?;
        for f in &mut m.files {
            for u in &mut f.units {
                u.file = f.path.clone();
            }
        }
        Ok(m)
    }

    pub(crate) fn to_text(&self) -> String {
        toml::to_string(self).expect("manifest serializes")
    }

    /// Replace everything `path` defined with `units`; the file moves to the end.
    pub(crate) fn replace_file(
        &mut self,
        path: PathBuf,
        hash: String,
        stamp: Option<Stamp>,
        std: String,
        mut units: Vec<UnitRecord>,
    ) {
        self.files.retain(|f| f.path != path);
        for u in &mut units {
            u.file = path.clone();
        }
        for f in &mut self.files {
            f.units
                .retain(|o| !units.iter().any(|u| u.kind == o.kind && u.key == o.key));
        }
        self.files.retain(|f| !f.units.is_empty());
        self.files.push(FileEntry {
            path,
            hash,
            std,
            size: stamp.map(|s| s.size),
            mtime_ns: stamp.map(|s| s.mtime_ns),
            units,
        });
    }

    pub fn units(&self) -> impl Iterator<Item = &UnitRecord> {
        self.files.iter().flat_map(|f| f.units.iter())
    }

    pub fn find(&self, kind: UnitKind, key: &str) -> Option<&UnitRecord> {
        self.units().find(|u| u.kind == kind && u.key == key)
    }

    pub fn file(&self, path: &std::path::Path) -> Option<&FileEntry> {
        self.files.iter().find(|f| f.path == path)
    }
}

/// Reads the text `Manifest::to_text` writes, about ten times faster than the `toml` crate (a
/// manifest of a library of a few hundred units is parsed by every run). `None` for anything else
/// (another format version, an escape, an unknown key, a missing field, hand editing): the caller
/// then parses with `toml`, which gives the same manifest or the diagnostic.
fn parse_written(text: &str) -> Option<Manifest> {
    enum At {
        Top,
        File,
        Unit,
        Dep,
    }
    fn string(v: &str) -> Option<&str> {
        let v = v.strip_prefix('"')?.strip_suffix('"')?;
        (!v.bytes().any(|b| b == b'"' || b == b'\\' || b < b' ')).then_some(v)
    }
    fn set(slot: &mut Option<String>, v: &str) -> Option<()> {
        if slot.is_some() {
            return None;
        }
        *slot = Some(string(v)?.to_string());
        Some(())
    }
    fn num<T: std::str::FromStr>(slot: &mut Option<T>, v: &str) -> Option<()> {
        if slot.is_some() {
            return None;
        }
        *slot = Some(v.parse().ok()?);
        Some(())
    }
    let mut format: Option<u32> = None;
    let mut name: Option<String> = None;
    let mut files: Vec<FileEntry> = Vec::new();
    // Fields of the file, unit and dependency being read; moved into their parents when done.
    let mut at = At::Top;
    let (mut path, mut hash, mut std): (Option<String>, Option<String>, Option<String>) =
        Default::default();
    let (mut size, mut mtime): (Option<u64>, Option<i64>) = Default::default();
    let (mut kind, mut uname, mut key): (Option<String>, Option<String>, Option<String>) =
        Default::default();
    let (mut ustd, mut primary): (Option<String>, Option<String>) = Default::default();
    let (mut dlib, mut dkind, mut dkey): (Option<String>, Option<String>, Option<String>) =
        Default::default();
    let mut units: Vec<UnitRecord> = Vec::new();
    let mut deps: Vec<UnitRef> = Vec::new();
    let kind_of = |k: &str| -> Option<UnitKind> {
        Some(match k {
            "entity" => UnitKind::Entity,
            "architecture" => UnitKind::Architecture,
            "package" => UnitKind::Package,
            "package_body" => UnitKind::PackageBody,
            "package_instance" => UnitKind::PackageInstance,
            "configuration" => UnitKind::Configuration,
            "context" => UnitKind::Context,
            _ => return None,
        })
    };
    // Closes the dependency / unit / file in progress.
    macro_rules! end_dep {
        () => {
            if let (At::Dep, Some(l), Some(k), Some(key)) =
                (&at, dlib.take(), dkind.take(), dkey.take())
            {
                deps.push(UnitRef {
                    library: l,
                    kind: kind_of(&k)?,
                    key,
                });
            } else if matches!(at, At::Dep) {
                return None;
            }
        };
    }
    macro_rules! end_unit {
        () => {
            end_dep!();
            if matches!(at, At::Unit | At::Dep) {
                units.push(UnitRecord {
                    kind: kind_of(&kind.take()?)?,
                    name: uname.take()?,
                    key: key.take()?,
                    primary: primary.take(),
                    std: ustd.take()?,
                    depends_on: std::mem::take(&mut deps),
                    file: PathBuf::new(),
                });
            }
        };
    }
    macro_rules! end_file {
        () => {
            end_unit!();
            if !matches!(at, At::Top) {
                files.push(FileEntry {
                    path: PathBuf::from(path.take()?),
                    hash: hash.take()?,
                    std: std.take()?,
                    size: size.take(),
                    mtime_ns: mtime.take(),
                    units: std::mem::take(&mut units),
                });
            }
        };
    }
    for line in text.lines() {
        if line.is_empty() {
            continue;
        }
        match line {
            "[[file]]" => {
                end_file!();
                at = At::File;
            }
            "[[file.unit]]" if !matches!(at, At::Top) => {
                end_unit!();
                at = At::Unit;
            }
            "[[file.unit.depends_on]]" if matches!(at, At::Unit | At::Dep) => {
                end_dep!();
                at = At::Dep;
            }
            _ if line.starts_with('[') => return None,
            _ => {
                let (k, v) = line.split_once(" = ")?;
                match (&at, k) {
                    (At::Top, "format") => num(&mut format, v)?,
                    (At::Top, "name") => set(&mut name, v)?,
                    (At::Top, "file") if v == "[]" => {}
                    (At::File, "unit") if v == "[]" => {}
                    (At::File, "path") => set(&mut path, v)?,
                    (At::File, "hash") => set(&mut hash, v)?,
                    (At::File, "std") => set(&mut std, v)?,
                    (At::File, "size") => num(&mut size, v)?,
                    (At::File, "mtime_ns") => num(&mut mtime, v)?,
                    (At::Unit, "kind") => set(&mut kind, v)?,
                    (At::Unit, "name") => set(&mut uname, v)?,
                    (At::Unit, "key") => set(&mut key, v)?,
                    (At::Unit, "primary") => set(&mut primary, v)?,
                    (At::Unit, "std") => set(&mut ustd, v)?,
                    (At::Unit, "depends_on") if v == "[]" => {}
                    (At::Dep, "library") => set(&mut dlib, v)?,
                    (At::Dep, "kind") => set(&mut dkind, v)?,
                    (At::Dep, "key") => set(&mut dkey, v)?,
                    _ => return None,
                }
            }
        }
    }
    end_file!();
    if format? != FORMAT {
        return None;
    }
    for f in &mut files {
        for u in &mut f.units {
            u.file = f.path.clone();
        }
    }
    Some(Manifest {
        format: FORMAT,
        name: name?,
        files,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample(path: &str) -> Manifest {
        let mut m = Manifest::new("lib");
        let a = UnitRecord::new(UnitKind::Package, "Pkg_A", "2008");
        let b =
            UnitRecord::package_body("Pkg_A", "2008").depends("lib", UnitKind::Package, "pkg_a");
        let c = UnitRecord::architecture("Ent", "Rtl", "2019")
            .depends("ieee", UnitKind::Package, "std_logic_1164")
            .depends("lib", UnitKind::Package, "pkg_a");
        m.replace_file(
            path.into(),
            "sha256:ab".into(),
            None,
            "2008".into(),
            vec![a, b],
        );
        m.replace_file(
            "/x/b.vhd".into(),
            "sha256:cd".into(),
            Some(Stamp {
                size: 12,
                mtime_ns: 1_700_000_000_123_456_789,
            }),
            "2019".into(),
            vec![c, UnitRecord::new(UnitKind::Context, "ctx", "2019")],
        );
        m
    }

    fn by_toml(text: &str) -> Manifest {
        let mut m: Manifest = toml::from_str(text).unwrap();
        for f in &mut m.files {
            for u in &mut f.units {
                u.file = f.path.clone();
            }
        }
        m
    }

    #[test]
    fn fast_parse_equals_toml() {
        let m = sample("/x/a.vhd");
        let text = m.to_text();
        assert_eq!(parse_written(&text), Some(by_toml(&text)));
        assert_eq!(
            Manifest::parse(&text, "m".as_ref()).unwrap(),
            by_toml(&text)
        );
        let empty = Manifest::new("empty").to_text();
        assert_eq!(parse_written(&empty), Some(by_toml(&empty)));
    }

    #[test]
    fn unusual_text_goes_to_toml() {
        // Quotes, backslashes and non-ASCII are written with escapes or literal strings: the fast
        // reader steps aside and the result is the same as ever.
        for path in [
            "/x/a\"b.vhd",
            "C:\\x\\a.vhd",
            "/x/\u{e9}.vhd",
            "/x/tab\there.vhd",
        ] {
            let text = sample(path).to_text();
            let got = Manifest::parse(&text, "m".as_ref()).unwrap();
            assert_eq!(got, by_toml(&text), "{path}");
            assert_eq!(got.files[0].path, PathBuf::from(path));
        }
        // Other format versions, unknown keys and missing fields are not the fast reader's.
        for text in [
            "format = 1\nname = \"l\"\n",
            "format = 2\nname = \"l\"\nextra = 1\n",
            "format = 2\n",
            "format = 2\nname = \"l\"\n[[file]]\npath = \"/a\"\n",
        ] {
            assert_eq!(parse_written(text), None, "{text}");
        }
    }
}

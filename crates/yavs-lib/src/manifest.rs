use crate::text::{Cur, put_line, put_num, put_str};
use crate::{Result, error::err};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};

pub const MANIFEST_FILE: &str = "yavs-lib.toml";
pub const LOCK_FILE: &str = "_YAVS_LIB";
/// The directory of the per-file records, next to the index.
pub const RECORD_DIR: &str = "yavs-lib.d";
pub(crate) const FORMAT: u32 = 3;

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

impl UnitKind {
    /// The name in the index and the records (the serde name).
    pub(crate) fn text(self) -> &'static str {
        match self {
            UnitKind::Entity => "entity",
            UnitKind::Architecture => "architecture",
            UnitKind::Package => "package",
            UnitKind::PackageBody => "package_body",
            UnitKind::PackageInstance => "package_instance",
            UnitKind::Configuration => "configuration",
            UnitKind::Context => "context",
        }
    }

    pub(crate) fn from_text(t: &str) -> Option<Self> {
        Some(match t {
            "entity" => UnitKind::Entity,
            "architecture" => UnitKind::Architecture,
            "package" => UnitKind::Package,
            "package_body" => UnitKind::PackageBody,
            "package_instance" => UnitKind::PackageInstance,
            "configuration" => UnitKind::Configuration,
            "context" => UnitKind::Context,
            _ => return None,
        })
    }
}

/// A string of the index: where it is in [`Manifest::arena`].
#[derive(Clone, Copy, Debug)]
struct Span(u32, u32);

/// A source file of the library, as the index knows it.
#[derive(Clone, Debug)]
struct IFile {
    path: Span,
    hash: Span,
    std: Span,
    size: Option<u64>,
    mtime_ns: Option<i64>,
    /// A later file replaced some units this file defined (#577): the index lists only the rest.
    partial: bool,
    /// The record (`RECORD_DIR`) of the file, named by this digest of its text.
    record: u128,
    /// Where the text of the file's entry is in [`Manifest::src`] while the entry is as written
    /// there: what [`Manifest::render`] copies instead of writing the entry again.
    block: Option<(u32, u32)>,
}

/// A unit the index knows, with the position of its file in `files`.
#[derive(Clone, Debug)]
struct IUnit {
    file: u32,
    kind: UnitKind,
    key: Span,
    primary: Option<Span>,
}

/// The index of a library (`yavs-lib.toml`): its files in analysis order with what a lookup needs
/// (where a unit is, the standard and content hash of a file, which units a file still defines),
/// and a digest naming the record of each file. A record holds the rest of a file's units (their
/// names and dependencies, `Manifest::records`) and is read only by those who need it. All
/// strings are in one buffer, so a parse makes a few allocations and a clone is a copy (#599).
#[derive(Clone, Debug, Default)]
pub struct Manifest {
    name: String,
    /// The library directory (holds `RECORD_DIR`); empty for a library that is not on disk.
    dir: PathBuf,
    arena: String,
    files: Vec<IFile>,
    /// In analysis order: the units of the first file, then of the second, ...
    units: Vec<IUnit>,
    /// The text of the index on disk, which `files[..].block` point into (empty for a library
    /// that is not on disk).
    src: String,
}

/// The index text of a manifest and where the entry of each file is in it.
pub(crate) struct Rendered {
    pub(crate) text: String,
    blocks: Vec<Option<(u32, u32)>>,
}

/// A source file of the library.
#[derive(Clone, Copy, Debug)]
pub struct FileRef<'a> {
    m: &'a Manifest,
    i: usize,
    /// Absolute path as analyzed.
    pub path: &'a Path,
    pub hash: &'a str,
    /// VHDL standard this file was analyzed with (per file: projects may mix 2008 and 2019).
    pub std: &'a str,
    /// A later file replaced some units this file defined: [`FileRef::units`] lists only the rest,
    /// so the file cannot be re-registered from them without parsing it (#577).
    pub partial: bool,
    size: Option<u64>,
    mtime_ns: Option<i64>,
}

impl FileRef<'_> {
    /// The file on disk is the one `hash` was taken of, by size and modification time alone.
    pub fn unchanged(&self) -> bool {
        match (self.size, self.mtime_ns) {
            (Some(size), Some(mtime_ns)) => Stamp::of(self.path) == Some(Stamp { size, mtime_ns }),
            _ => false,
        }
    }

    /// The units the file still defines, in the order they were analyzed.
    pub fn units(&self) -> impl Iterator<Item = UnitLoc<'_>> {
        let i = self.i as u32;
        self.m
            .units
            .iter()
            .filter(move |u| u.file == i)
            .map(|u| self.m.loc(u))
    }
}

/// A unit and the file that defines it.
#[derive(Clone, Copy, Debug)]
pub struct UnitLoc<'a> {
    pub kind: UnitKind,
    /// Folded lookup key (architectures: `entity/arch`).
    pub key: &'a str,
    /// Folded key of the entity (architecture) or package (body) this unit belongs to.
    pub primary: Option<&'a str>,
    pub file: &'a Path,
    /// The standard of the file.
    pub std: &'a str,
    /// Position of the file in the analysis order.
    pub(crate) file_idx: usize,
}

impl Manifest {
    pub(crate) fn new(name: &str, dir: &Path) -> Self {
        Manifest {
            name: name.into(),
            dir: dir.into(),
            ..Self::default()
        }
    }

    fn s(&self, sp: Span) -> &str {
        &self.arena[sp.0 as usize..sp.1 as usize]
    }

    fn add(&mut self, s: &str) -> Span {
        let at = self.arena.len() as u32;
        self.arena.push_str(s);
        Span(at, self.arena.len() as u32)
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    /// Number of files.
    pub fn len(&self) -> usize {
        self.files.len()
    }

    pub fn is_empty(&self) -> bool {
        self.files.is_empty()
    }

    pub(crate) fn file_at(&self, i: usize) -> FileRef<'_> {
        let f = &self.files[i];
        FileRef {
            m: self,
            i,
            path: Path::new(self.s(f.path)),
            hash: self.s(f.hash),
            std: self.s(f.std),
            partial: f.partial,
            size: f.size,
            mtime_ns: f.mtime_ns,
        }
    }

    fn loc(&self, u: &IUnit) -> UnitLoc<'_> {
        let f = &self.files[u.file as usize];
        UnitLoc {
            kind: u.kind,
            key: self.s(u.key),
            primary: u.primary.map(|p| self.s(p)),
            file: Path::new(self.s(f.path)),
            std: self.s(f.std),
            file_idx: u.file as usize,
        }
    }

    /// The files in analysis order.
    pub fn files(&self) -> impl Iterator<Item = FileRef<'_>> {
        (0..self.files.len()).map(|i| self.file_at(i))
    }

    fn index_of_str(&self, t: &str) -> Option<usize> {
        (0..self.files.len()).find(|&i| t == self.s(self.files[i].path))
    }

    fn index_of(&self, path: &Path) -> Option<usize> {
        // The text of the path first (it is what the index holds), else by components.
        path.to_str()
            .and_then(|t| self.index_of_str(t))
            .or_else(|| {
                (0..self.files.len()).find(|&i| Path::new(self.s(self.files[i].path)) == path)
            })
    }

    pub fn file(&self, path: &Path) -> Option<FileRef<'_>> {
        self.index_of(path).map(|i| self.file_at(i))
    }

    /// Every unit, in analysis order.
    pub fn units(&self) -> impl Iterator<Item = UnitLoc<'_>> {
        self.units.iter().map(|u| self.loc(u))
    }

    pub fn find(&self, kind: UnitKind, key: &str) -> Option<UnitLoc<'_>> {
        self.units
            .iter()
            .find(|u| u.kind == kind && self.s(u.key) == key)
            .map(|u| self.loc(u))
    }

    /// Read the index text `render` wrote. A library of another format version is `L0004`.
    #[cfg(test)]
    pub(crate) fn parse(text: &str, origin: &Path) -> Result<Self> {
        Self::parse_owned(text.to_owned(), origin)
    }

    /// [`Manifest::parse`] of a text the caller has no more use for: the manifest keeps it.
    pub(crate) fn parse_owned(text: String, origin: &Path) -> Result<Self> {
        let first = text.lines().next().unwrap_or("");
        if let Some(v) = first.strip_prefix("format = ")
            && let Ok(v) = v.parse::<u32>()
            && v != FORMAT
        {
            return Err(Box::new(
                err(
                    "L0004",
                    format!("library manifest `{}` has format version {v}, this yavs reads version {FORMAT}", origin.display()),
                )
                .with_help(format!(
                    "delete the library directory `{}` and analyze its files again (a library of another format version is not converted)",
                    origin.parent().unwrap_or(Path::new("")).display()
                )),
            ));
        }
        let mut m = parse_index(&text, origin.parent().unwrap_or(Path::new(""))).map_err(
            |(line, why)| {
                Box::new(
                    err(
                        "L0003",
                        format!("library manifest `{}` is corrupt", origin.display()),
                    )
                    .with_note(format!("line {line}: {why}"))
                    .with_help(format!(
                        "delete the library directory `{}` and analyze its files again",
                        origin.parent().unwrap_or(Path::new("")).display()
                    )),
                )
            },
        )?;
        m.src = text;
        Ok(m)
    }

    /// The text of the index on disk, as last read or [adopted](Manifest::adopt).
    pub(crate) fn src(&self) -> &str {
        &self.src
    }

    /// The index text of the manifest as it is now. The entry of a file that is as it was in
    /// [`Manifest::src`] is copied from there, so writing the index of a library of hundreds of
    /// files after one more file costs a copy and one entry (debug builds also write the whole
    /// text and compare).
    pub(crate) fn render(&self) -> Rendered {
        let mut o = String::with_capacity(self.src.len() + 1024);
        put_num(&mut o, "format", FORMAT);
        put_line(&mut o, "name", &self.name);
        let mut blocks = Vec::with_capacity(self.files.len());
        let mut at = 0;
        for (i, f) in self.files.iter().enumerate() {
            let start = o.len() as u32;
            match f.block {
                Some((a, b)) => {
                    o.push_str(&self.src[a as usize..b as usize]);
                    // The units of the file are together, in file order.
                    while self.units.get(at).is_some_and(|u| u.file as usize == i) {
                        at += 1;
                    }
                }
                None => self.put_file(&mut o, i, f, &mut at),
            }
            blocks.push(Some((start, o.len() as u32)));
        }
        debug_assert_eq!(o, self.render_whole(), "a spliced index is the whole one");
        Rendered { text: o, blocks }
    }

    /// The index text with every entry written afresh.
    fn render_whole(&self) -> String {
        let mut o = String::with_capacity(self.files.len() * 400 + self.units.len() * 60 + 64);
        put_num(&mut o, "format", FORMAT);
        put_line(&mut o, "name", &self.name);
        let mut at = 0;
        for (i, f) in self.files.iter().enumerate() {
            self.put_file(&mut o, i, f, &mut at);
        }
        o
    }

    /// The entry of file `i`; `at` is the position of its first unit and moves past its units.
    fn put_file(&self, o: &mut String, i: usize, f: &IFile, at: &mut usize) {
        o.push_str("\n[[file]]\n");
        put_line(o, "path", self.s(f.path));
        put_line(o, "hash", self.s(f.hash));
        put_line(o, "std", self.s(f.std));
        if let Some(n) = f.size {
            put_num(o, "size", n);
        }
        if let Some(n) = f.mtime_ns {
            put_num(o, "mtime_ns", n);
        }
        if f.partial {
            o.push_str("partial = true\n");
        }
        put_line(o, "record", &format!("{:032x}", f.record));
        o.push_str("units = [");
        // The units of a file are together, in file order.
        while let Some(u) = self.units.get(*at).filter(|u| u.file as usize == i) {
            if *at > 0 && self.units[*at - 1].file as usize == i {
                o.push_str(", ");
            }
            o.push('[');
            put_str(o, u.kind.text());
            o.push_str(", ");
            put_str(o, self.s(u.key));
            if let Some(p) = u.primary {
                o.push_str(", ");
                put_str(o, self.s(p));
            }
            o.push(']');
            *at += 1;
        }
        o.push_str("]\n");
    }

    /// Take `r` for the text on disk (it was written): the entries are in it from now on.
    pub(crate) fn adopt(&mut self, r: Rendered) {
        for (f, b) in self.files.iter_mut().zip(r.blocks) {
            f.block = b;
        }
        self.src = r.text;
    }

    /// `render` as text (tests).
    #[cfg(test)]
    pub(crate) fn to_text(&self) -> String {
        self.render().text
    }

    /// The digest of the record of `path`.
    pub(crate) fn record_of(&self, path: &str) -> Option<u128> {
        self.index_of_str(path).map(|i| self.files[i].record)
    }

    /// The digests of the records the index names.
    pub(crate) fn record_digests(&self) -> impl Iterator<Item = u128> + '_ {
        self.files.iter().map(|f| f.record)
    }

    /// Forget `path`; the digest of the record it had.
    pub(crate) fn remove_file(&mut self, path: &str) -> Option<u128> {
        let i = self.index_of_str(path)?;
        Some(self.remove_at(i))
    }

    fn remove_at(&mut self, i: usize) -> u128 {
        let f = self.files.remove(i);
        self.units.retain(|u| u.file as usize != i);
        for u in &mut self.units {
            if u.file as usize > i {
                u.file -= 1;
            }
        }
        f.record
    }

    /// Take the units `new` define away from the files that define them now: those files are
    /// `partial`, and one left with no unit is forgotten. The digests of the records of the
    /// forgotten files.
    pub(crate) fn take_units(&mut self, new: &[UnitRecord]) -> Vec<u128> {
        let mut lost: Vec<u32> = Vec::new();
        let (arena, files) = (&self.arena, &mut self.files);
        self.units.retain(|u| {
            let key = &arena[u.key.0 as usize..u.key.1 as usize];
            let hit = new.iter().any(|n| n.kind == u.kind && n.key == key);
            if hit && !lost.contains(&u.file) {
                lost.push(u.file);
                files[u.file as usize].partial = true;
                files[u.file as usize].block = None;
            }
            !hit
        });
        lost.sort_unstable_by(|a, b| b.cmp(a));
        let mut gone = Vec::new();
        for i in lost {
            if !self.units.iter().any(|u| u.file == i) {
                gone.push(self.remove_at(i as usize));
            }
        }
        gone
    }

    /// Add `path` at the end of the analysis order.
    pub(crate) fn push_file(
        &mut self,
        path: &str,
        hash: &str,
        std: &str,
        stamp: Option<Stamp>,
        record: u128,
        units: &[UnitRecord],
    ) {
        let file = self.files.len() as u32;
        let f = IFile {
            path: self.add(path),
            hash: self.add(hash),
            std: self.add(std),
            size: stamp.map(|s| s.size),
            mtime_ns: stamp.map(|s| s.mtime_ns),
            partial: false,
            record,
            block: None,
        };
        self.files.push(f);
        for u in units {
            let (key, primary) = (self.add(&u.key), u.primary.as_deref().map(|p| self.add(p)));
            self.units.push(IUnit {
                file,
                kind: u.kind,
                key,
                primary,
            });
        }
    }

    /// The directory of the records of this library.
    pub(crate) fn record_dir(&self) -> PathBuf {
        self.dir.join(RECORD_DIR)
    }

    /// The full records of the units of `f` (names and dependencies), read from the record the
    /// index names. The record can hold units the file lost since (`partial`).
    pub fn records(&self, f: &FileRef<'_>) -> Result<Vec<UnitRecord>> {
        crate::record::load(&self.record_dir(), self.files[f.i].record, f.path)
    }
}

/// The parse of [`Manifest::parse`]: the line number and the reason on failure.
fn parse_index(text: &str, dir: &Path) -> std::result::Result<Manifest, (usize, &'static str)> {
    /// The fields of the file being read.
    #[derive(Default)]
    struct Pending {
        path: Option<Span>,
        hash: Option<Span>,
        std: Option<Span>,
        size: Option<u64>,
        mtime_ns: Option<i64>,
        partial: bool,
        record: Option<u128>,
        units: bool,
        /// Where the entry starts in the text: at the blank line before `[[file]]`, if there is one.
        start: Option<u32>,
        /// Where the line of the units ends.
        end: u32,
    }
    let b = text.as_bytes();
    // The line a byte is on (only asked for on an error).
    let line_of = |pos: usize| {
        1 + b[..pos.min(b.len())]
            .iter()
            .filter(|&&c| c == b'\n')
            .count()
    };
    let mut m = Manifest {
        dir: dir.into(),
        arena: String::with_capacity(text.len()),
        ..Manifest::default()
    };
    if text.is_empty() {
        return Err((1, "expected `format = N` first"));
    }
    let first_end = text.find('\n').unwrap_or(b.len());
    let first = &text[..first_end];
    if first
        .strip_suffix('\r')
        .unwrap_or(first)
        .strip_prefix("format = ")
        .and_then(|v| v.parse::<u32>().ok())
        != Some(FORMAT)
    {
        return Err((1, "expected `format = N` first"));
    }
    // The start of the line after the one that ends at `from`: a newline with a carriage return
    // before it allowed, or the end of the text.
    let eol = |from: usize| -> Option<usize> {
        match b.get(from) {
            None => Some(from),
            Some(b'\n') => Some(from + 1),
            Some(b'\r') if b.get(from + 1) == Some(&b'\n') => Some(from + 2),
            _ => None,
        }
    };
    let mut pos = (first_end + 1).min(b.len());
    let mut at: Option<Pending> = None;
    let mut name = false;
    // The entry being read is complete, and the next one (or the end of the text) is at `$next`.
    macro_rules! finish {
        ($line:expr, $next:expr) => {
            if let Some(p) = at.take() {
                let (Some(path), Some(hash), Some(std), Some(record), true) =
                    (p.path, p.hash, p.std, p.record, p.units)
                else {
                    return Err((line_of($line), "a file lacks a field"));
                };
                // Its text can be copied when it is the whole entry up to the next one.
                let block = p
                    .start
                    .filter(|_| p.end as usize == $next)
                    .map(|s| (s, p.end));
                m.files.push(IFile {
                    path,
                    hash,
                    std,
                    size: p.size,
                    mtime_ns: p.mtime_ns,
                    partial: p.partial,
                    record,
                    block,
                });
            }
        };
    }
    while pos < b.len() {
        let line_start = pos;
        let n = || line_of(line_start);
        if b[pos] == b'\n' {
            pos += 1;
            continue;
        }
        let rest = &text[pos..];
        if rest.starts_with("[[file]]")
            && let Some(next) = eol(pos + 8)
        {
            // The blank line before the entry belongs to it (`put_file` starts with it).
            let blank = line_start >= 1 && b[line_start - 1] == b'\n';
            finish!(line_start, if blank { line_start - 1 } else { line_start });
            at = Some(Pending {
                start: blank.then(|| line_start as u32 - 1),
                ..Pending::default()
            });
            pos = next;
            continue;
        }
        // `key = ` of a line.
        let key_end = rest
            .bytes()
            .position(|c| c == b' ' || c == b'\n')
            .filter(|&i| b[pos + i] == b' ');
        let Some(key_end) = key_end.filter(|&i| rest[i..].starts_with(" = ")) else {
            return Err((n(), "expected `key = value`"));
        };
        let key = &rest[..key_end];
        let vstart = pos + key_end + 3;
        let mut c = Cur::new(&text[vstart..]);
        // A string value, into the buffer; the line ends after it.
        macro_rules! string {
            () => {{
                let start = m.arena.len() as u32;
                c.string(&mut m.arena).ok_or_else(|| (n(), "bad string"))?;
                pos = eol(vstart + c.pos()).ok_or_else(|| (n(), "text after the value"))?;
                Span(start, m.arena.len() as u32)
            }};
        }
        // The text of the value up to the end of the line, for the numbers.
        let line_value = || {
            let e = text[vstart..].find('\n').map_or(b.len(), |i| vstart + i);
            text[vstart..e]
                .strip_suffix('\r')
                .unwrap_or(&text[vstart..e])
        };
        match (&mut at, key) {
            (None, "name") if !name => {
                let mut s = String::new();
                c.string(&mut s).ok_or_else(|| (n(), "bad string"))?;
                m.name = s;
                name = true;
                pos = eol(vstart + c.pos()).ok_or_else(|| (n(), "text after the value"))?;
            }
            (Some(p), "path") => p.path = Some(string!()),
            (Some(p), "hash") => p.hash = Some(string!()),
            (Some(p), "std") => p.std = Some(string!()),
            (Some(p), "size") => {
                p.size = Some(digits(line_value()).ok_or_else(|| (n(), "bad number"))?);
                pos = vstart + line_value().len();
                pos = eol(pos).ok_or_else(|| (n(), "bad number"))?;
            }
            (Some(p), "mtime_ns") => {
                p.mtime_ns = Some(
                    digits(line_value())
                        .and_then(|v| i64::try_from(v).ok())
                        .ok_or_else(|| (n(), "bad number"))?,
                );
                pos = vstart + line_value().len();
                pos = eol(pos).ok_or_else(|| (n(), "bad number"))?;
            }
            (Some(p), "partial") if line_value() == "true" => {
                p.partial = true;
                pos = eol(vstart + 4).ok_or_else(|| (n(), "unexpected line"))?;
            }
            (Some(p), "record") => {
                let sp = string!();
                p.record = Some(
                    hex128(&m.arena[sp.0 as usize..sp.1 as usize])
                        .ok_or_else(|| (n(), "bad digest"))?,
                );
            }
            (Some(p), "units") if !p.units => {
                p.units = true;
                let file = m.files.len() as u32;
                let sep = |c: &mut Cur| c.eat(b',').and_then(|()| c.eat(b' '));
                c.eat(b'[').ok_or_else(|| (n(), "expected ["))?;
                while c.peek() != Some(b']') {
                    c.eat(b'[').ok_or_else(|| (n(), "expected ["))?;
                    let kind = {
                        let k = c.plain().ok_or_else(|| (n(), "bad string"))?;
                        UnitKind::from_text(k).ok_or_else(|| (n(), "unknown unit kind"))?
                    };
                    sep(&mut c).ok_or_else(|| (n(), "expected a comma"))?;
                    let start = m.arena.len() as u32;
                    c.string(&mut m.arena).ok_or_else(|| (n(), "bad string"))?;
                    let ukey = Span(start, m.arena.len() as u32);
                    let primary = if c.peek() == Some(b',') {
                        sep(&mut c).ok_or_else(|| (n(), "expected a comma"))?;
                        let start = m.arena.len() as u32;
                        c.string(&mut m.arena).ok_or_else(|| (n(), "bad string"))?;
                        Some(Span(start, m.arena.len() as u32))
                    } else {
                        None
                    };
                    c.eat(b']').ok_or_else(|| (n(), "expected ]"))?;
                    m.units.push(IUnit {
                        file,
                        kind,
                        key: ukey,
                        primary,
                    });
                    if c.peek() == Some(b',') {
                        sep(&mut c).ok_or_else(|| (n(), "expected a comma"))?;
                    }
                }
                c.eat(b']').ok_or_else(|| (n(), "expected ]"))?;
                pos = eol(vstart + c.pos()).ok_or_else(|| (n(), "text after the units"))?;
                if let Some(p) = at.as_mut() {
                    p.end = pos as u32;
                }
            }
            _ => return Err((n(), "unexpected line")),
        }
    }
    finish!(b.len(), b.len());
    if !name {
        return Err((text.lines().count(), "no name"));
    }
    Ok(m)
}

/// The value of `s`, decimal digits only.
fn digits(s: &str) -> Option<u64> {
    if s.is_empty() {
        return None;
    }
    s.bytes().try_fold(0u64, |v, c| {
        let d = c.checked_sub(b'0').filter(|&d| d < 10)?;
        v.checked_mul(10)?.checked_add(u64::from(d))
    })
}

/// The value of `s`, 1 to 32 hexadecimal digits.
fn hex128(s: &str) -> Option<u128> {
    if s.is_empty() || s.len() > 32 {
        return None;
    }
    s.bytes().try_fold(0u128, |v, c| {
        Some(v << 4 | u128::from((c as char).to_digit(16)?))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample(path: &str) -> Manifest {
        let mut m = Manifest::new("lib", Path::new("/l"));
        let a = UnitRecord::new(UnitKind::Package, "Pkg_A", "2008");
        let b = UnitRecord::package_body("Pkg_A", "2008");
        let c = UnitRecord::architecture("Ent", "Rtl", "2019");
        m.push_file(path, "sha256:ab", "2008", None, 7, &[a, b]);
        let stamp = Some(Stamp {
            size: 12,
            mtime_ns: 1_700_000_000_123_456_789,
        });
        m.push_file(
            "/x/b.vhd",
            "sha256:cd",
            "2019",
            stamp,
            u128::MAX,
            &[c, UnitRecord::new(UnitKind::Context, "ctx", "2019")],
        );
        m
    }

    fn same(a: &Manifest, b: &Manifest) {
        assert_eq!(a.name(), b.name());
        let files = |m: &Manifest| {
            m.files()
                .map(|f| {
                    (
                        f.path.to_path_buf(),
                        f.hash.to_string(),
                        f.std.to_string(),
                        f.partial,
                        f.size,
                        f.mtime_ns,
                        f.m.files[f.i].record,
                    )
                })
                .collect::<Vec<_>>()
        };
        assert_eq!(files(a), files(b));
        let units = |m: &Manifest| {
            m.units()
                .map(|u| {
                    (
                        u.kind,
                        u.key.to_string(),
                        u.primary.map(String::from),
                        u.file.to_path_buf(),
                    )
                })
                .collect::<Vec<_>>()
        };
        assert_eq!(units(a), units(b));
    }

    #[test]
    fn text_round_trips() {
        for path in [
            "/x/a.vhd",
            "/x/a\"b.vhd",
            "C:\\x\\a.vhd",
            "/x/\u{e9}.vhd",
            "/x/tab\there.vhd",
            "/x/n\nl.vhd",
            "/x/it's #1 [a] = b.vhd",
        ] {
            let m = sample(path);
            let text = m.to_text();
            assert!(text.starts_with("format = 3\nname = \"lib\"\n"));
            let back = Manifest::parse(&text, Path::new("/l/yavs-lib.toml")).unwrap();
            same(&m, &back);
            // Written again, the same text.
            assert_eq!(back.to_text(), text, "{path}");
            assert_eq!(back.file(Path::new(path)).unwrap().path, Path::new(path));
        }
        let empty = Manifest::new("empty", Path::new("/l"));
        let back = Manifest::parse(&empty.to_text(), Path::new("/l/yavs-lib.toml")).unwrap();
        assert!(back.is_empty() && back.name() == "empty");
    }

    #[test]
    fn parsed_text_is_the_toml_of_the_same_values() {
        // The text stays valid TOML, for anyone who looks at it.
        let text = sample("/x/it's \"q\" \\ \u{e9}.vhd").to_text();
        let v: toml::Value = toml::from_str(&text).unwrap();
        assert_eq!(v["format"].as_integer(), Some(3));
        assert_eq!(
            v["file"][0]["path"].as_str(),
            Some("/x/it's \"q\" \\ \u{e9}.vhd")
        );
        assert_eq!(v["file"][1]["units"][0][1].as_str(), Some("ent/rtl"));
        assert_eq!(v["file"][1]["units"][0][2].as_str(), Some("ent"));
    }

    #[test]
    fn partial_and_removed_files() {
        let mut m = sample("/x/a.vhd");
        // c.vhd defines package pkg_a again: a.vhd keeps only its body and is partial.
        let gone = m.take_units(&[UnitRecord::new(UnitKind::Package, "Pkg_A", "2008")]);
        assert!(gone.is_empty());
        m.push_file(
            "/x/c.vhd",
            "sha256:ef",
            "2008",
            None,
            9,
            &[UnitRecord::new(UnitKind::Package, "Pkg_A", "2008")],
        );
        let partial: Vec<_> = m
            .files()
            .map(|f| (f.path.to_path_buf(), f.partial))
            .collect();
        assert_eq!(
            partial,
            [
                ("/x/a.vhd".into(), true),
                ("/x/b.vhd".into(), false),
                ("/x/c.vhd".into(), false)
            ]
        );
        // It is written, read back and a file written again is whole.
        let back = Manifest::parse(&m.to_text(), Path::new("/l/yavs-lib.toml")).unwrap();
        same(&m, &back);
        assert!(back.file(Path::new("/x/a.vhd")).unwrap().partial);
        assert_eq!(m.remove_file("/x/a.vhd"), Some(7));
        m.push_file(
            "/x/a.vhd",
            "sha256:ab",
            "2008",
            None,
            7,
            &[UnitRecord::package_body("Pkg_A", "2008")],
        );
        assert!(m.files().all(|f| !f.partial));
        // A file that lost every unit goes, with the digest of its record.
        let gone = m.take_units(&[
            UnitRecord::new(UnitKind::Context, "ctx", "2019"),
            UnitRecord::architecture("Ent", "Rtl", "2019"),
        ]);
        assert_eq!(gone, [u128::MAX]);
        assert!(m.file(Path::new("/x/b.vhd")).is_none());
        assert_eq!(m.units().count(), 2);
    }

    #[test]
    fn a_rendered_index_copies_the_entries_that_did_not_change() {
        let p = Path::new("/l/yavs-lib.toml");
        let mut m = Manifest::parse(&sample("/x/a.vhd").to_text(), p).unwrap();
        // Parsed from canonical text: both entries are copied.
        assert!(m.files.iter().all(|f| f.block.is_some()));
        let text = m.src().to_string();
        assert_eq!(m.render().text, text);
        // A file that lost a unit is written afresh, the other one copied; a new file is appended.
        m.take_units(&[UnitRecord::new(UnitKind::Package, "Pkg_A", "2008")]);
        assert!(m.files[0].block.is_none() && m.files[1].block.is_some());
        m.push_file("/x/c.vhd", "sha256:ef", "2008", None, 9, &[]);
        let r = m.render();
        assert!(r.text.contains("partial = true") && r.text.contains("/x/c.vhd"));
        assert_eq!(r.text, m.render_whole());
        let back = Manifest::parse(&r.text, p).unwrap();
        same(&m, &back);
        // Once the text is adopted every entry is a copy, also after a file moves to the end.
        m.adopt(r);
        assert!(m.files.iter().all(|f| f.block.is_some()));
        let before = m.render().text;
        m.remove_file("/x/b.vhd");
        m.push_file("/x/b.vhd", "sha256:cd", "2019", None, 5, &[]);
        let after = m.render();
        assert_ne!(after.text, before);
        assert_eq!(after.text, m.render_whole());
        m.adopt(after);
        // Text that is not canonical (no blank line between entries) is still read; its entries
        // are written afresh where they cannot be copied.
        let tight = m.src().replace("\n\n[[file]]", "\n[[file]]");
        let t = Manifest::parse(&tight, p).unwrap();
        assert_eq!(t.render().text, t.render_whole());
        same(&m, &t);
    }

    #[test]
    fn corrupt_text_is_named() {
        let good = sample("/x/a.vhd").to_text();
        let p = Path::new("/l/yavs-lib.toml");
        let line_of = |text: &str| -> String {
            let e = Manifest::parse(text, p).unwrap_err();
            let d = crate::render(&e, false);
            assert!(
                d.contains("L0003") && d.contains("`/l/yavs-lib.toml` is corrupt"),
                "{d}"
            );
            d
        };
        assert!(line_of("").contains("line 1"));
        assert!(line_of("name = \"x\"\n").contains("line 1"));
        assert!(line_of(&good.replace("name = \"lib\"\n", "")).contains("no name"));
        assert!(line_of(&good.replace("std = \"2008\"", "std = 2008")).contains("bad string"));
        assert!(line_of(&good.replace("record = ", "recor = ")).contains("unexpected line"));
        assert!(
            line_of(&good.replace("[\"package\",", "[\"packge\",")).contains("unknown unit kind")
        );
        assert!(line_of(&good.replace("\"]]\n", "\"]] x\n")).contains("text after the units"));
        assert!(line_of(&good.replace("units = ", "units = [] ")).contains("text after"));
        let cut = &good[..good.len() - 7];
        let d = line_of(cut);
        assert!(d.contains("line"), "{d}");
        assert!(line_of(&good.replace("size = 12", "size = 1x")).contains("bad number"));
        assert!(line_of(&good.replace("size = 12", "size = ")).contains("bad number"));
        assert!(line_of(&good.replace("size = 12", "size = -12")).contains("bad number"));
        assert!(line_of(&good.replace("mtime_ns = 17", "mtime_ns = x7")).contains("bad number"));
        assert!(
            line_of(&good.replace("record = ", "partial = false\nrecord = "))
                .contains("unexpected")
        );
        // A string that is not closed, or has a bad escape.
        assert!(
            line_of(&good.replace("hash = \"sha256:ab\"", "hash = \"sha256:ab"))
                .contains("bad string")
        );
        assert!(
            line_of(&good.replace("hash = \"sha256:ab\"", "hash = \"a\\qb\""))
                .contains("bad string")
        );
        // Another format version is not corrupt.
        let e = Manifest::parse(&good.replacen("format = 3", "format = 4", 1), p).unwrap_err();
        assert!(crate::render(&e, false).contains("L0004"));
    }
}

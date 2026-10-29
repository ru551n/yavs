//! Diagnostics model shared by every yavs stage.
//!
//! A [`Diagnostic`] has a severity, a stable code (such as `E0001`), a message,
//! an optional primary [`Label`], labelled secondary spans, and `note:`/`help:`
//! lines. [`render`] turns one into text in one of three [`MessageFormat`]s:
//! rustc-style snippets (via `annotate-snippets`), one line per diagnostic, or
//! one JSON object per line.
//!
//! # Code registry
//!
//! Every diagnostic code is unique across yavs. Each stage owns a range and
//! defines its codes as `pub const`s next to the code that reports them.
//!
//! | Range | Owner |
//! |---|---|
//! | `E0001`-`E0099`, `W0001`-`W0099` | `yavs-syntax`: lexing, conditional analysis, parsing (`yavs_syntax::codes`) |
//! | `E0100`-`E0199`, `W0100`-`W0199` | `yavs` command line: reading files, arguments, `--load` libraries |
//! | `E1000`-`E1999`, `W1000`-`W1999` | `yavs-sem`: semantic analysis (`yavs_sem::diag`, ranges in `docs/design/sem.md` 8.1; E1999 not implemented yet) |
//! | `E2100`-`E2199`, `W2100`-`W2199` | `yavs-elab`: elaboration (`yavs_elab::codes`) |
//! | `E0700`-`E0799`, `W0700`-`W0799` | `yavs-dap`: the Debug Adapter Protocol server (E0701, E0702 in use) |
//! | `E0900`-`E0999`, `W0900`-`W0999` | `yavs-vhpi` (E0901, E0902, W0903, W0904, E0905 in use) |
//! | `E3000`-`E3099` | `yavs-kernel`: simulation runtime errors, one per `ErrorKind` (`yavs_kernel::ErrorKind::code`) |
//! | `W3100`-`W3199` | run-time warnings of `yavs run` about a stuck or hung simulation (`yavs_elab::stuck`: W3101 hang report, W3102 process that never suspends) |
//! | `L0000`-`L0099` | `yavs-lib`: library manifests and closure queries |
//! | `E2000`-`E2099` | `yavs-lower`: HIR to IR lowering (E2001 not lowered yet) |
//! | `E9999` | internal compiler error in any stage ("this is a bug in yavs") |
//!
//! A code is never reused for a different meaning. An empty code is only for
//! summary lines such as "aborting due to 2 previous errors" and the informational note
//! that gives the seed of a shuffled run.

mod shared;

pub use shared::{Buffer, Slice, sharing};

use std::fmt::Write as _;
use std::io::IsTerminal;

use annotate_snippets::{AnnotationKind, Group, Level, Origin, Renderer, Snippet};

/// Index of a file in a [`SourceMap`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Default)]
pub struct FileId(pub u32);

/// Maps file ids while values are written or read (see [`map_file_ids`]); `None` fails the write
/// or read.
pub type FileIdMap = std::rc::Rc<dyn Fn(FileId) -> Option<FileId>>;

thread_local! {
    static FILE_MAP: std::cell::RefCell<Option<FileIdMap>> = const { std::cell::RefCell::new(None) };
}

/// Run `f` with every [`FileId`] written or read passed through `map`: the analysed-unit cache
/// stores a unit's spans with file numbers of its own and turns them into the ids of the session
/// that reads it.
pub fn map_file_ids<R>(map: FileIdMap, f: impl FnOnce() -> R) -> R {
    let before = FILE_MAP.with(|m| m.borrow_mut().replace(map));
    let r = f();
    FILE_MAP.with(|m| *m.borrow_mut() = before);
    r
}

/// Run `f` with file ids written and read as they are, inside a [`map_file_ids`].
pub fn unmapped_file_ids<R>(f: impl FnOnce() -> R) -> R {
    let before = FILE_MAP.with(|m| m.borrow_mut().take());
    let r = f();
    FILE_MAP.with(|m| *m.borrow_mut() = before);
    r
}

fn mapped_file(id: FileId) -> Option<Option<FileId>> {
    FILE_MAP.with(|m| m.borrow().as_ref().map(|m| m(id)))
}

impl serde::Serialize for FileId {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        match mapped_file(*self) {
            None => s.serialize_newtype_struct("FileId", &self.0),
            Some(Some(id)) => s.serialize_newtype_struct("FileId", &id.0),
            Some(None) => Err(serde::ser::Error::custom("file id outside the map")),
        }
    }
}

impl<'de> serde::Deserialize<'de> for FileId {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        struct V;
        impl<'de> serde::de::Visitor<'de> for V {
            type Value = u32;
            fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
                f.write_str("a file id")
            }
            fn visit_newtype_struct<D: serde::Deserializer<'de>>(
                self,
                d: D,
            ) -> Result<u32, D::Error> {
                <u32 as serde::Deserialize>::deserialize(d)
            }
        }
        let id = FileId(d.deserialize_newtype_struct("FileId", V)?);
        match mapped_file(id) {
            None => Ok(id),
            Some(Some(id)) => Ok(id),
            Some(None) => Err(serde::de::Error::custom("file id outside the map")),
        }
    }
}

/// A byte range in one source file.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct Span {
    pub file: FileId,
    pub start: u32,
    pub end: u32,
}

/// The file number the default span (no place) is written as under [`map_file_ids`]: its
/// `FileId(0)` names no file.
const NO_FILE: u32 = u32::MAX;

impl serde::Serialize for Span {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        use serde::ser::SerializeStruct;
        let mut t = s.serialize_struct("Span", 3)?;
        if *self == Span::default() && FILE_MAP.with(|m| m.borrow().is_some()) {
            t.serialize_field("file", &NO_FILE)?;
        } else {
            t.serialize_field("file", &self.file)?;
        }
        t.serialize_field("start", &self.start)?;
        t.serialize_field("end", &self.end)?;
        t.end()
    }
}

impl<'de> serde::Deserialize<'de> for Span {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        struct V;
        impl<'de> serde::de::Visitor<'de> for V {
            type Value = Span;
            fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
                f.write_str("a span")
            }
            fn visit_seq<A: serde::de::SeqAccess<'de>>(self, mut a: A) -> Result<Span, A::Error> {
                use serde::de::Error;
                let mapped = FILE_MAP.with(|m| m.borrow().is_some());
                let file = if mapped {
                    let raw: u32 = a.next_element()?.ok_or_else(|| A::Error::custom("span"))?;
                    if raw == NO_FILE {
                        FileId(0)
                    } else {
                        mapped_file(FileId(raw))
                            .flatten()
                            .ok_or_else(|| A::Error::custom("file id outside the map"))?
                    }
                } else {
                    a.next_element()?.ok_or_else(|| A::Error::custom("span"))?
                };
                let start = a.next_element()?.ok_or_else(|| A::Error::custom("span"))?;
                let end = a.next_element()?.ok_or_else(|| A::Error::custom("span"))?;
                Ok(Span { file, start, end })
            }
        }
        d.deserialize_struct("Span", &["file", "start", "end"], V)
    }
}

impl Span {
    pub fn new(file: FileId, start: u32, end: u32) -> Self {
        Span { file, start, end }
    }

    /// The smallest span covering both `self` and `other` (same file).
    pub fn to(self, other: Span) -> Span {
        Span::new(
            self.file,
            self.start.min(other.start),
            self.end.max(other.end),
        )
    }
}

impl std::fmt::Debug for Span {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}..{}", self.start, self.end)
    }
}

/// One loaded source file.
///
/// Spans are byte offsets into the file as it is on disk. For a UTF-8 file
/// that is the same as an offset into `text`; for a file read as ISO-8859-1
/// the original bytes are kept and offsets are mapped when rendering.
///
/// A file restored from an analysis image keeps no text: only the path, the line starts and a
/// digest of the bytes. The text is read again the first time something needs it (a rendered
/// snippet, a re-scan of a construct), so a run without such a need never decodes or reads it.
pub struct SourceFile {
    pub path: String,
    line_starts: Vec<u32>,
    full: std::sync::OnceLock<Result<Full, Unavailable>>,
    /// Set for a file restored from an image: how to read it again.
    lazy: Option<Lazy>,
}

struct Full {
    /// The decoded text, for display.
    text: String,
    /// The bytes on disk, kept only for a Latin-1 file.
    raw: Option<Vec<u8>>,
    /// File offsets of the bytes of a Latin-1 file that are two bytes in `text`.
    high: Vec<u32>,
}

/// Why the text of a file restored from an image cannot be shown.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Unavailable {
    /// The file no longer has the bytes the image was made from.
    Changed,
    /// The file cannot be read.
    Unreadable,
}

impl Unavailable {
    /// The note a diagnostic carries in place of its snippet.
    pub fn note(self, path: &str) -> String {
        match self {
            Unavailable::Changed => {
                format!("source of {path} changed since it was analysed; snippet omitted")
            }
            Unavailable::Unreadable => {
                format!("source of {path} could not be read; snippet omitted")
            }
        }
    }
}

struct Lazy {
    latin1: bool,
    digest: u64,
    read: SourceReader,
}

/// Reads the bytes of a file by the path recorded in the map.
pub type SourceReader = fn(&str) -> std::io::Result<Vec<u8>>;

/// The digest of a file's bytes recorded in an image.
pub fn digest(bytes: &[u8]) -> u64 {
    twox_hash::XxHash3_64::oneshot(bytes)
}

impl Full {
    fn new(text: String, raw: Option<Vec<u8>>) -> Full {
        let high = raw
            .iter()
            .flatten()
            .enumerate()
            .filter(|&(_, &b)| b >= 0x80)
            .map(|(i, _)| i as u32)
            .collect();
        Full { text, raw, high }
    }
}

fn latin1_text(bytes: &[u8]) -> String {
    bytes.iter().map(|&b| b as char).collect()
}

impl SourceFile {
    fn load(&self) -> Result<&Full, Unavailable> {
        self.full
            .get_or_init(|| {
                let l = self.lazy.as_ref().expect("an eager file is always loaded");
                let bytes = (l.read)(&self.path).map_err(|_| Unavailable::Unreadable)?;
                if digest(&bytes) != l.digest {
                    return Err(Unavailable::Changed);
                }
                Ok(if l.latin1 {
                    Full::new(latin1_text(&bytes), Some(bytes))
                } else {
                    let text = String::from_utf8(bytes)
                        .expect("a UTF-8 file stays UTF-8 (digest checked)");
                    Full::new(text, None)
                })
            })
            .as_ref()
            .map_err(|e| *e)
    }

    /// The loaded file, for the callers that need the text and cannot go on without it (a
    /// re-scan of a construct); a diagnostic uses [`SourceFile::try_text`] instead.
    fn full(&self) -> &Full {
        self.load()
            .unwrap_or_else(|e| panic!("internal error: {}", e.note(&self.path)))
    }

    /// The decoded text, or why it cannot be shown.
    pub fn try_text(&self) -> Result<&str, Unavailable> {
        self.load().map(|f| f.text.as_str())
    }

    fn digest(&self) -> u64 {
        self.lazy
            .as_ref()
            .map_or_else(|| digest(self.bytes()), |l| l.digest)
    }

    /// The decoded text, for display.
    pub fn text(&self) -> &str {
        &self.full().text
    }

    /// The file as it is on disk.
    pub fn bytes(&self) -> &[u8] {
        let f = self.full();
        f.raw.as_deref().unwrap_or(f.text.as_bytes())
    }

    /// Offsets of the first byte of each line, in `text`: those of the file on disk unless
    /// [`SourceFile::is_latin1`].
    pub fn line_starts(&self) -> &[u32] {
        &self.line_starts
    }

    /// The file was read as ISO-8859-1 bytes rather than UTF-8.
    pub fn is_latin1(&self) -> bool {
        self.lazy
            .as_ref()
            .map_or_else(|| self.full().raw.is_some(), |l| l.latin1)
    }

    /// The offset in `text` of a file byte offset, on a character boundary.
    pub fn text_offset(&self, offset: u32) -> usize {
        let f = self.full();
        let mut t = (offset as usize + f.high.partition_point(|&h| h < offset)).min(f.text.len());
        while !f.text.is_char_boundary(t) {
            t -= 1;
        }
        t
    }

    /// 1-based line and column (in characters) of a file byte offset. When the text of a
    /// restored file is unavailable, the line comes from the stored line starts and the column
    /// counts bytes (the 3 above is a byte column).
    pub fn line_col(&self, offset: u32) -> (usize, usize) {
        let Ok(f) = self.load() else {
            let line = self.line_starts.partition_point(|&s| s <= offset).max(1) - 1;
            return (line + 1, (offset - self.line_starts[line]) as usize + 1);
        };
        let t = self.text_offset(offset);
        let line = self
            .line_starts
            .partition_point(|&s| s as usize <= t)
            .max(1)
            - 1;
        let start = self.line_starts[line] as usize;
        let col = f
            .text
            .get(start..t)
            .map_or(t - start, |s| s.chars().count());
        (line + 1, col + 1)
    }
}

/// What an image holds of a file: no text (see [`SourceFile`]), only what locates and verifies it.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct FileStub {
    pub path: String,
    pub line_starts: Vec<u32>,
    pub latin1: bool,
    pub digest: u64,
}

/// All files a session has loaded, addressed by [`FileId`].
#[derive(Default)]
pub struct SourceMap {
    files: Vec<SourceFile>,
}

impl serde::Serialize for SourceMap {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.collect_seq(self.files.iter().map(SourceFile::stub))
    }
}

impl<'de> serde::Deserialize<'de> for SourceMap {
    /// The files come back unloaded, to be read with `std::fs::read` unless
    /// [`SourceMap::set_reader`] says otherwise.
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let stubs = Vec::<FileStub>::deserialize(d)?;
        let files = stubs
            .into_iter()
            .map(|f| SourceFile::unloaded(f, |p| std::fs::read(p)))
            .collect();
        Ok(SourceMap { files })
    }
}

impl SourceFile {
    fn unloaded(f: FileStub, read: SourceReader) -> SourceFile {
        SourceFile {
            path: f.path,
            line_starts: f.line_starts,
            full: std::sync::OnceLock::new(),
            lazy: Some(Lazy {
                latin1: f.latin1,
                digest: f.digest,
                read,
            }),
        }
    }

    /// What an image keeps of this file.
    pub fn stub(&self) -> FileStub {
        FileStub {
            path: self.path.clone(),
            line_starts: self.line_starts.clone(),
            latin1: self.is_latin1(),
            digest: self.digest(),
        }
    }
}

impl SourceMap {
    /// Add a UTF-8 file.
    pub fn add(&mut self, path: impl Into<String>, text: impl Into<String>) -> FileId {
        self.push(path.into(), text.into(), None)
    }

    /// Add a file whose bytes are ISO-8859-1 characters.
    pub fn add_latin1(&mut self, path: impl Into<String>, bytes: Vec<u8>) -> FileId {
        let text = latin1_text(&bytes);
        self.push(path.into(), text, Some(bytes))
    }

    fn push(&mut self, path: String, text: String, raw: Option<Vec<u8>>) -> FileId {
        let line_starts = std::iter::once(0)
            .chain(text.match_indices('\n').map(|(i, _)| i as u32 + 1))
            .collect();
        self.files.push(SourceFile {
            path,
            line_starts,
            full: std::sync::OnceLock::from(Ok(Full::new(text, raw))),
            lazy: None,
        });
        FileId(self.files.len() as u32 - 1)
    }

    /// How files restored from an image are read again (embedded library files, say).
    pub fn set_reader(&mut self, read: SourceReader) {
        for l in self.files.iter_mut().filter_map(|f| f.lazy.as_mut()) {
            l.read = read;
        }
    }

    /// Add a file known only by its stub; its text is read with `read` when first needed.
    pub fn add_stub(&mut self, stub: FileStub, read: SourceReader) -> FileId {
        self.files.push(SourceFile::unloaded(stub, read));
        FileId(self.files.len() as u32 - 1)
    }

    pub fn len(&self) -> usize {
        self.files.len()
    }

    pub fn is_empty(&self) -> bool {
        self.files.is_empty()
    }

    /// Forget the files added after the first `n` (an analysed-unit cache read that is undone).
    pub fn truncate(&mut self, n: usize) {
        self.files.truncate(n);
    }

    pub fn get(&self, id: FileId) -> &SourceFile {
        &self.files[id.0 as usize]
    }

    /// Every file as (path, digest of its bytes on disk), in `FileId` order; loads nothing.
    pub fn digests(&self) -> impl Iterator<Item = (&str, u64)> {
        self.files.iter().map(|f| (f.path.as_str(), f.digest()))
    }
}

#[derive(
    Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, serde::Serialize, serde::Deserialize,
)]
pub enum Severity {
    Note,
    Warning,
    Error,
}

impl Severity {
    fn as_str(self) -> &'static str {
        match self {
            Severity::Error => "error",
            Severity::Warning => "warning",
            Severity::Note => "note",
        }
    }
}

/// A span with an (optionally empty) message.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Label {
    pub span: Span,
    pub message: String,
}

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize)]
pub struct Diagnostic {
    pub severity: Severity,
    /// Stable code, for `--explain` and per-code suppression; empty for
    /// summary lines such as "aborting due to 2 previous errors".
    pub code: &'static str,
    pub message: String,
    pub primary: Option<Label>,
    pub secondary: Vec<Label>,
    pub notes: Vec<String>,
    pub help: Vec<String>,
}

/// What a diagnostic is read from: the code as text, interned so it can be `'static` again.
#[derive(serde::Deserialize)]
struct DiagnosticRepr {
    severity: Severity,
    code: String,
    message: String,
    primary: Option<Label>,
    secondary: Vec<Label>,
    notes: Vec<String>,
    help: Vec<String>,
}

impl<'de> serde::Deserialize<'de> for Diagnostic {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        use std::collections::HashSet;
        use std::sync::{Mutex, OnceLock};
        static SEEN: OnceLock<Mutex<HashSet<&'static str>>> = OnceLock::new();
        let r = DiagnosticRepr::deserialize(d)?;
        let mut seen = SEEN
            .get_or_init(Default::default)
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let code = match seen.get(r.code.as_str()) {
            Some(&k) => k,
            None => {
                let k: &'static str = Box::leak(r.code.into_boxed_str());
                seen.insert(k);
                k
            }
        };
        Ok(Diagnostic {
            severity: r.severity,
            code,
            message: r.message,
            primary: r.primary,
            secondary: r.secondary,
            notes: r.notes,
            help: r.help,
        })
    }
}

impl Diagnostic {
    pub fn new(severity: Severity, code: &'static str, message: impl Into<String>) -> Self {
        Diagnostic {
            severity,
            code,
            message: message.into(),
            primary: None,
            secondary: Vec::new(),
            notes: Vec::new(),
            help: Vec::new(),
        }
    }

    pub fn error(code: &'static str, message: impl Into<String>) -> Self {
        Self::new(Severity::Error, code, message)
    }

    pub fn warning(code: &'static str, message: impl Into<String>) -> Self {
        Self::new(Severity::Warning, code, message)
    }

    pub fn with_primary(mut self, span: Span, message: impl Into<String>) -> Self {
        self.primary = Some(Label {
            span,
            message: message.into(),
        });
        self
    }

    pub fn with_secondary(mut self, span: Span, message: impl Into<String>) -> Self {
        self.secondary.push(Label {
            span,
            message: message.into(),
        });
        self
    }

    pub fn with_note(mut self, note: impl Into<String>) -> Self {
        self.notes.push(note.into());
        self
    }

    pub fn with_help(mut self, help: impl Into<String>) -> Self {
        self.help.push(help.into());
        self
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum MessageFormat {
    #[default]
    Human,
    Short,
    Json,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum ColorChoice {
    #[default]
    Auto,
    Always,
    Never,
}

impl ColorChoice {
    /// Whether to colour output to stderr: `auto` means a TTY and no `NO_COLOR`.
    pub fn use_color(self) -> bool {
        match self {
            ColorChoice::Always => true,
            ColorChoice::Never => false,
            ColorChoice::Auto => {
                std::env::var_os("NO_COLOR").is_none_or(|v| v.is_empty())
                    && std::io::stderr().is_terminal()
            }
        }
    }
}

/// Render one diagnostic. The result ends with a newline.
pub fn render(
    sources: &SourceMap,
    diag: &Diagnostic,
    format: MessageFormat,
    color: bool,
) -> String {
    match format {
        MessageFormat::Human => render_human(sources, diag, color),
        MessageFormat::Short => render_short(sources, diag),
        MessageFormat::Json => render_json(sources, diag),
    }
}

fn level(severity: Severity) -> Level<'static> {
    match severity {
        Severity::Error => Level::ERROR,
        Severity::Warning => Level::WARNING,
        Severity::Note => Level::NOTE,
    }
}

fn render_human(sources: &SourceMap, diag: &Diagnostic, color: bool) -> String {
    let mut title = level(diag.severity).primary_title(diag.message.as_str());
    if !diag.code.is_empty() {
        title = title.id(diag.code);
    }
    let mut group = Group::with_title(title);
    let mut omitted = Vec::new();
    // One snippet per file, primary file first.
    let mut files: Vec<FileId> = Vec::new();
    for l in diag.primary.iter().chain(&diag.secondary) {
        if !files.contains(&l.span.file) {
            files.push(l.span.file);
        }
    }
    for file in files {
        let src = sources.get(file);
        if let Err(e) = src.try_text() {
            omitted.push((file, e));
            continue;
        }
        // Hand over only the annotated lines, not the whole file.
        let labels = || {
            diag.primary
                .iter()
                .chain(&diag.secondary)
                .filter(move |l| l.span.file == file)
        };
        let len = src.text().len();
        let lo = labels()
            .map(|l| src.text_offset(l.span.start))
            .min()
            .unwrap_or(0);
        let hi = labels()
            .map(|l| src.text_offset(l.span.end))
            .max()
            .unwrap_or(0)
            .max(lo);
        let lo = src.text()[..lo].rfind('\n').map_or(0, |i| i + 1);
        let hi = src.text()[hi..].find('\n').map_or(len, |i| hi + i);
        let first_line = src.line_starts.partition_point(|&s| s as usize <= lo);
        let mut snippet = Snippet::source(&src.text()[lo..hi])
            .line_start(first_line)
            .path(src.path.as_str());
        let clamp = |s: Span| {
            src.text_offset(s.start).clamp(lo, hi) - lo..src.text_offset(s.end).clamp(lo, hi) - lo
        };
        for l in diag.primary.iter().filter(|l| l.span.file == file) {
            snippet = snippet.annotation(
                AnnotationKind::Primary
                    .span(clamp(l.span))
                    .label(non_empty(&l.message)),
            );
        }
        for l in diag.secondary.iter().filter(|l| l.span.file == file) {
            snippet = snippet.annotation(
                AnnotationKind::Context
                    .span(clamp(l.span))
                    .label(non_empty(&l.message)),
            );
        }
        group = group.element(snippet);
    }
    // Text unavailable: the location, the labels and why, as notes in place of the snippet.
    let mut gone = Vec::new();
    for &(file, e) in &omitted {
        let src = sources.get(file);
        let labels = diag
            .primary
            .iter()
            .map(|l| (l, true))
            .chain(diag.secondary.iter().map(|l| (l, false)));
        let mut labels = labels.filter(|(l, _)| l.span.file == file).peekable();
        if let Some((l, _)) = labels.peek() {
            let (line, col) = src.line_col(l.span.start);
            group = group.element(Origin::path(src.path.as_str()).line(line).char_column(col));
        }
        for (l, primary) in labels.filter(|(l, _)| !l.message.is_empty()) {
            let (line, col) = src.line_col(l.span.start);
            let kind = if primary { "label" } else { "secondary label" };
            gone.push(format!("{kind} at {line}:{col}: {}", l.message));
        }
        gone.push(e.note(&src.path));
    }
    for n in gone.iter().chain(&diag.notes) {
        group = group.element(Level::NOTE.message(n.as_str()));
    }
    for h in &diag.help {
        group = group.element(Level::HELP.message(h.as_str()));
    }
    let renderer = if color {
        Renderer::styled()
    } else {
        Renderer::plain()
    };
    let mut out = renderer.render(&[group]);
    out.push_str("\n\n");
    out
}

fn non_empty(s: &str) -> Option<&str> {
    (!s.is_empty()).then_some(s)
}

fn location(sources: &SourceMap, span: Span) -> (&str, usize, usize) {
    let f = sources.get(span.file);
    let (line, col) = f.line_col(span.start);
    (&f.path, line, col)
}

fn render_short(sources: &SourceMap, diag: &Diagnostic) -> String {
    let sev = diag.severity.as_str();
    // A diagnostic without a code (a plain note) has no brackets.
    let code = if diag.code.is_empty() {
        String::new()
    } else {
        format!("[{}]", diag.code)
    };
    match &diag.primary {
        Some(l) => {
            let (path, line, col) = location(sources, l.span);
            format!("{path}:{line}:{col}: {sev}{code}: {}\n", diag.message)
        }
        None => format!("{sev}{code}: {}\n", diag.message),
    }
}

fn json_str(out: &mut String, s: &str) {
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => {
                let _ = write!(out, "\\u{:04x}", c as u32);
            }
            c => out.push(c),
        }
    }
    out.push('"');
}

fn render_json(sources: &SourceMap, diag: &Diagnostic) -> String {
    let mut o = String::from("{\"level\":");
    json_str(&mut o, diag.severity.as_str());
    o.push_str(",\"code\":");
    json_str(&mut o, diag.code);
    o.push_str(",\"message\":");
    json_str(&mut o, &diag.message);
    o.push_str(",\"spans\":[");
    let labels = diag
        .primary
        .iter()
        .map(|l| (l, true))
        .chain(diag.secondary.iter().map(|l| (l, false)));
    for (i, (l, primary)) in labels.enumerate() {
        if i > 0 {
            o.push(',');
        }
        let (path, line, col) = location(sources, l.span);
        let (end_line, end_col) = sources.get(l.span.file).line_col(l.span.end);
        o.push_str("{\"file\":");
        json_str(&mut o, path);
        let _ = write!(
            o,
            ",\"byte_start\":{},\"byte_end\":{},\"line_start\":{line},\"column_start\":{col},\
             \"line_end\":{end_line},\"column_end\":{end_col},\"primary\":{primary},\"label\":",
            l.span.start, l.span.end
        );
        json_str(&mut o, &l.message);
        o.push('}');
    }
    // rustc-compatible children: notes then help, each a childless diagnostic.
    o.push_str("],\"children\":[");
    let kids = diag
        .notes
        .iter()
        .map(|m| ("note", m))
        .chain(diag.help.iter().map(|m| ("help", m)));
    for (i, (level, m)) in kids.enumerate() {
        if i > 0 {
            o.push(',');
        }
        let _ = write!(o, "{{\"level\":\"{level}\",\"message\":");
        json_str(&mut o, m);
        o.push_str(",\"spans\":[],\"children\":[],\"rendered\":null}");
    }
    o.push_str("],\"rendered\":");
    json_str(&mut o, &render_human(sources, diag, false));
    o.push_str("}\n");
    o
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> (SourceMap, Diagnostic) {
        let mut sm = SourceMap::default();
        let f = sm.add("a.vhd", "entity e is\nend entity f;\n");
        let d = Diagnostic::error("E0001", "mismatched name")
            .with_primary(Span::new(f, 23, 24), "expected `e`")
            .with_secondary(Span::new(f, 7, 8), "entity declared here")
            .with_help("use `end entity e;`");
        (sm, d)
    }

    #[test]
    fn line_col() {
        let (sm, _) = sample();
        let f = sm.get(FileId(0));
        assert_eq!(f.line_col(0), (1, 1));
        assert_eq!(f.line_col(12), (2, 1));
        assert_eq!(f.line_col(23), (2, 12));
    }

    /// A map written to an image keeps no text; the files are read again when first needed and
    /// render exactly as before.
    #[test]
    fn restored_files_load_on_demand() {
        let tmp = tempfile::tempdir().expect("dir");
        let dir = tmp.path();
        let utf8 = dir.join("u.vhd");
        let latin1 = dir.join("l.vhd");
        std::fs::write(&utf8, "a\u{e9}b\nentity e is\n").expect("write");
        std::fs::write(&latin1, b"a\xe9b\nx\xe9 y\n").expect("write");
        let mut sm = SourceMap::default();
        let u = sm.add(utf8.display().to_string(), "a\u{e9}b\nentity e is\n");
        let l = sm.add_latin1(latin1.display().to_string(), b"a\xe9b\nx\xe9 y\n".to_vec());
        let bytes = postcard::to_stdvec(&sm).expect("serialize");
        let paths = utf8.display().to_string().len() + latin1.display().to_string().len();
        assert!(
            bytes.len() < paths + 100,
            "no text in the image: {} bytes",
            bytes.len()
        );
        let back: SourceMap = postcard::from_bytes(&bytes).expect("deserialize");
        assert!(back.get(u).full.get().is_none() && back.get(l).full.get().is_none());
        assert_eq!(back.get(l).line_starts(), sm.get(l).line_starts());
        assert!(back.get(l).is_latin1() && !back.get(u).is_latin1());
        assert_eq!(
            back.digests().collect::<Vec<_>>(),
            sm.digests().collect::<Vec<_>>()
        );
        for (id, off) in [(u, 7), (l, 7)] {
            let d =
                Diagnostic::error("E0001", "m").with_primary(Span::new(id, off, off + 1), "here");
            assert_eq!(
                render(&back, &d, MessageFormat::Human, false),
                render(&sm, &d, MessageFormat::Human, false)
            );
        }
        assert!(back.get(u).full.get().is_some());
        // A file that changed since the image was made is never shown as the old one: the
        // diagnostic loses its snippet but keeps code, location, labels and a note.
        std::fs::write(&utf8, "changed\n").expect("write");
        let back: SourceMap = postcard::from_bytes(&bytes).expect("deserialize");
        let d = Diagnostic::error("E0001", "m")
            .with_primary(Span::new(u, 7, 8), "here")
            .with_secondary(Span::new(u, 0, 1), "there");
        let path = utf8.display().to_string();
        let human = render(&back, &d, MessageFormat::Human, false);
        let want = format!(
            "error[E0001]: m\n --> {path}:2:3\n  = note: label at 2:3: here\n  = note: secondary label at 1:1: there\n  = note: source of {path} changed since it was analysed; snippet omitted\n\n"
        );
        assert_eq!(human, want);
        assert_eq!(
            render(&back, &d, MessageFormat::Short, false),
            format!("{path}:2:3: error[E0001]: m\n")
        );
        let j = render(&back, &d, MessageFormat::Json, false);
        assert!(j.contains("\"line_start\":2,\"column_start\":3"), "{j}");
        assert!(!j.contains("1 | "), "{j}");
        // An unreadable file likewise.
        std::fs::remove_file(&utf8).expect("remove");
        let back: SourceMap = postcard::from_bytes(&bytes).expect("deserialize");
        let human = render(&back, &d, MessageFormat::Human, false);
        assert!(
            human.contains("could not be read; snippet omitted") && !human.contains(" | "),
            "{human}"
        );
        assert_eq!(back.get(u).try_text(), Err(Unavailable::Unreadable));
    }

    #[test]
    fn latin1_offsets_are_file_bytes() {
        let mut sm = SourceMap::default();
        let f = sm.add_latin1("l.vhd", b"a\xe9b\nx\xe9 y\n".to_vec());
        assert_eq!(sm.get(f).text(), "a\u{e9}b\nx\u{e9} y\n");
        assert_eq!(sm.get(f).bytes()[7], b'y');
        assert_eq!(sm.get(f).line_col(7), (2, 4));
        let d = Diagnostic::error("E0001", "m").with_primary(Span::new(f, 7, 8), "here");
        let out = render(&sm, &d, MessageFormat::Human, false);
        assert!(out.contains("2 | x\u{e9} y\n  |    ^ here"), "{out}");
        let j = render(&sm, &d, MessageFormat::Json, false);
        assert!(
            j.contains("\"byte_start\":7,\"byte_end\":8,\"line_start\":2,\"column_start\":4"),
            "{j}"
        );
    }

    #[test]
    fn human() {
        let (sm, d) = sample();
        let out = render(&sm, &d, MessageFormat::Human, false);
        assert!(out.starts_with("error[E0001]: mismatched name\n"), "{out}");
        assert!(out.contains("--> a.vhd:2:12"), "{out}");
        assert!(out.contains("entity declared here"), "{out}");
        assert!(out.contains("= help: use `end entity e;`"), "{out}");
    }

    #[test]
    fn short_and_json() {
        let (sm, d) = sample();
        assert_eq!(
            render(&sm, &d, MessageFormat::Short, false),
            "a.vhd:2:12: error[E0001]: mismatched name\n"
        );
        let j = render(&sm, &d, MessageFormat::Json, false);
        assert!(
            j.starts_with("{\"level\":\"error\",\"code\":\"E0001\""),
            "{j}"
        );
        assert!(j.contains("\"line_start\":2,\"column_start\":12"), "{j}");
        assert!(
            j.contains("\"children\":[{\"level\":\"help\",\"message\":\"use `end entity e;`\""),
            "{j}"
        );
        assert!(
            j.contains("\"rendered\":\"error[E0001]: mismatched name\\n"),
            "{j}"
        );
    }
}

#[cfg(test)]
mod serde_tests {
    #[test]
    fn diagnostic_is_owned_deserialize() {
        fn owned<T: serde::de::DeserializeOwned>() {}
        owned::<super::Diagnostic>();
        owned::<super::SourceMap>();
    }
}

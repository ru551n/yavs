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
//! | `E1000`-`E1999`, `W1000`-`W1999` | `yavs-sem`: semantic analysis (`yavs_sem::diag`, ranges in `docs/design/sem.md` 8.1) |
//! | `E2100`-`E2199`, `W2100`-`W2199` | `yavs-elab`: elaboration (`yavs_elab::codes`) |
//! | `E0700`-`E0799`, `W0700`-`W0799` | `yavs-dap`: the Debug Adapter Protocol server (E0701 in use) |
//! | `E0900`-`E0999`, `W0900`-`W0999` | `yavs-vhpi` (E0901, E0902, W0903, W0904, E0905 in use) |
//! | `E3000`-`E3099` | `yavs-kernel`: simulation runtime errors, one per `ErrorKind` (`yavs_kernel::ErrorKind::code`) |
//! | `L0000`-`L0099` | `yavs-lib`: library manifests and closure queries |
//! | `E2000`-`E2099` | `yavs-lower`: HIR to IR lowering (E2001 not lowered yet) |
//! | `E9999` | internal compiler error in any stage ("this is a bug in yavs") |
//!
//! A code is never reused for a different meaning. An empty code is only for
//! summary lines such as "aborting due to 2 previous errors".

use std::fmt::Write as _;
use std::io::IsTerminal;

use annotate_snippets::{AnnotationKind, Group, Level, Renderer, Snippet};

/// Index of a file in a [`SourceMap`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Default)]
pub struct FileId(pub u32);

/// A byte range in one source file.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct Span {
    pub file: FileId,
    pub start: u32,
    pub end: u32,
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
pub struct SourceFile {
    pub path: String,
    /// The decoded text, for display.
    pub text: String,
    line_starts: Vec<u32>,
    /// The bytes on disk, kept only for a Latin-1 file.
    raw: Option<Vec<u8>>,
    /// File offsets of the bytes of a Latin-1 file that are two bytes in `text`.
    high: Vec<u32>,
}

impl SourceFile {
    /// The file as it is on disk.
    pub fn bytes(&self) -> &[u8] {
        self.raw.as_deref().unwrap_or(self.text.as_bytes())
    }

    /// The file was read as ISO-8859-1 bytes rather than UTF-8.
    pub fn is_latin1(&self) -> bool {
        self.raw.is_some()
    }

    /// The offset in `text` of a file byte offset, on a character boundary.
    pub fn text_offset(&self, offset: u32) -> usize {
        let mut t =
            (offset as usize + self.high.partition_point(|&h| h < offset)).min(self.text.len());
        while !self.text.is_char_boundary(t) {
            t -= 1;
        }
        t
    }

    /// 1-based line and column (in characters) of a file byte offset.
    pub fn line_col(&self, offset: u32) -> (usize, usize) {
        let offset = self.text_offset(offset) as u32;
        let line = self.line_starts.partition_point(|&s| s <= offset) - 1;
        let start = self.line_starts[line] as usize;
        let end = (offset as usize).min(self.text.len());
        let col = self
            .text
            .get(start..end)
            .map_or(end - start, |s| s.chars().count());
        (line + 1, col + 1)
    }
}

/// All files a session has loaded, addressed by [`FileId`].
#[derive(Default)]
pub struct SourceMap {
    files: Vec<SourceFile>,
}

impl SourceMap {
    /// Add a UTF-8 file.
    pub fn add(&mut self, path: impl Into<String>, text: impl Into<String>) -> FileId {
        self.push(path.into(), text.into(), None)
    }

    /// Add a file whose bytes are ISO-8859-1 characters.
    pub fn add_latin1(&mut self, path: impl Into<String>, bytes: Vec<u8>) -> FileId {
        let text = bytes.iter().map(|&b| b as char).collect();
        self.push(path.into(), text, Some(bytes))
    }

    fn push(&mut self, path: String, text: String, raw: Option<Vec<u8>>) -> FileId {
        let line_starts = std::iter::once(0)
            .chain(text.match_indices('\n').map(|(i, _)| i as u32 + 1))
            .collect();
        let high = raw
            .iter()
            .flatten()
            .enumerate()
            .filter(|&(_, &b)| b >= 0x80)
            .map(|(i, _)| i as u32)
            .collect();
        self.files.push(SourceFile {
            path,
            text,
            line_starts,
            raw,
            high,
        });
        FileId(self.files.len() as u32 - 1)
    }

    pub fn get(&self, id: FileId) -> &SourceFile {
        &self.files[id.0 as usize]
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
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
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Label {
    pub span: Span,
    pub message: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
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
    // One snippet per file, primary file first.
    let mut files: Vec<FileId> = Vec::new();
    for l in diag.primary.iter().chain(&diag.secondary) {
        if !files.contains(&l.span.file) {
            files.push(l.span.file);
        }
    }
    for file in files {
        let src = sources.get(file);
        // Hand over only the annotated lines, not the whole file.
        let labels = || {
            diag.primary
                .iter()
                .chain(&diag.secondary)
                .filter(move |l| l.span.file == file)
        };
        let len = src.text.len();
        let lo = labels()
            .map(|l| src.text_offset(l.span.start))
            .min()
            .unwrap_or(0);
        let hi = labels()
            .map(|l| src.text_offset(l.span.end))
            .max()
            .unwrap_or(0)
            .max(lo);
        let lo = src.text[..lo].rfind('\n').map_or(0, |i| i + 1);
        let hi = src.text[hi..].find('\n').map_or(len, |i| hi + i);
        let first_line = src.line_starts.partition_point(|&s| s as usize <= lo);
        let mut snippet = Snippet::source(&src.text[lo..hi])
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
    for n in &diag.notes {
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
    match &diag.primary {
        Some(l) => {
            let (path, line, col) = location(sources, l.span);
            format!(
                "{path}:{line}:{col}: {sev}[{}]: {}\n",
                diag.code, diag.message
            )
        }
        None => format!("{sev}[{}]: {}\n", diag.code, diag.message),
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

    #[test]
    fn latin1_offsets_are_file_bytes() {
        let mut sm = SourceMap::default();
        let f = sm.add_latin1("l.vhd", b"a\xe9b\nx\xe9 y\n".to_vec());
        assert_eq!(sm.get(f).text, "a\u{e9}b\nx\u{e9} y\n");
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

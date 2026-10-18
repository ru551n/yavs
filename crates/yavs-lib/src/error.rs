use std::path::Path;
use yavs_diag::{Diagnostic, Severity};

pub(crate) fn err(code: &'static str, msg: impl Into<String>) -> Diagnostic {
    Diagnostic::error(code, msg)
}

pub(crate) fn io(
    code: &'static str,
    what: &str,
    path: &Path,
    e: &std::io::Error,
) -> Box<Diagnostic> {
    Box::new(
        Diagnostic::error(code, format!("cannot {what} `{}`", path.display()))
            .with_note(e.to_string()),
    )
}

pub(crate) fn note(msg: String) {
    let d = Diagnostic::new(Severity::Note, "L0000", msg);
    eprint!("{}", crate::render(&d, false));
}

/// Debug-level note: only with YAVS_LOG=debug.
pub(crate) fn debug(msg: String) {
    if std::env::var("YAVS_LOG").is_ok_and(|v| v == "debug") {
        eprintln!("debug: {msg}");
    }
}

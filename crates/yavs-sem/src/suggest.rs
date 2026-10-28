//! "Did you mean" suggestions (sem.md 8.3).

use yavs_diag::{Diagnostic, Span};

use crate::db::Db;
use crate::decl::{DeclKind, Designator, ObjClass, ObjRole, SubprogramKind};
use crate::ids::DeclId;

impl Db {
    /// What the declaration is, for "a signal with a similar name exists".
    pub(crate) fn decl_noun(&self, d: DeclId) -> &'static str {
        match &self.decls[d].kind {
            DeclKind::Object(o) => match (&o.role, o.class) {
                (ObjRole::Port, _) => "port",
                (ObjRole::Generic, _) => "generic",
                (ObjRole::Param, _) => "parameter",
                (ObjRole::LoopParam | ObjRole::GenerateParam, _) => "loop parameter",
                (_, ObjClass::Constant) => "constant",
                (_, ObjClass::Signal) => "signal",
                (_, ObjClass::Variable) => "variable",
                (_, ObjClass::SharedVariable) => "shared variable",
                (_, ObjClass::File) => "file",
            },
            DeclKind::Subprogram(s) | DeclKind::GenericSubprogram(s) => match s.kind {
                SubprogramKind::Function { .. } => "function",
                SubprogramKind::Procedure => "procedure",
            },
            DeclKind::Type(_) | DeclKind::GenericType(_) => "type",
            DeclKind::Subtype(_) => "subtype",
            DeclKind::EnumLit { .. } => "enumeration literal",
            DeclKind::PhysUnit { .. } => "physical unit",
            DeclKind::Alias(_) => "alias",
            DeclKind::Attribute { .. } => "attribute",
            DeclKind::Component(_) => "component",
            DeclKind::Label(_) => "label",
            DeclKind::Library(_) => "library",
            DeclKind::Unit(_)
            | DeclKind::Package { .. }
            | DeclKind::PackageInst(_)
            | DeclKind::GenericPackage(_) => "design unit",
            _ => "name",
        }
    }

    /// Candidate for [`similar`]: identifier declarations only (not operators or characters).
    pub(crate) fn candidate(&self, d: DeclId) -> Option<(&str, DeclId)> {
        match &self.decls[d].name {
            Designator::Ident(i) => Some((i.key.as_str(), d)),
            _ => None,
        }
    }

    /// Add the suggestion for the declarations `found` to `diag` (primary span `at`): the
    /// help line and, for a single one in the same file, a label at its declaration.
    pub(crate) fn suggest_decls(
        &self,
        mut diag: Diagnostic,
        at: Span,
        found: &[DeclId],
    ) -> Diagnostic {
        if found.is_empty() {
            return diag;
        }
        let shown: Vec<(String, &str)> = found
            .iter()
            .map(|&d| (self.decls[d].name.to_string(), self.decl_noun(d)))
            .collect();
        diag = diag.with_help(help(&shown));
        if let [d] = found {
            let decl = &self.decls[*d];
            let span = decl.name_span();
            if !decl.implicit && span != Span::default() && span.file == at.file {
                diag = diag.with_secondary(span, "similar name declared here");
            }
        }
        diag
    }
}

/// Optimal string alignment distance (Damerau-Levenshtein with adjacent transpositions).
pub fn distance(a: &str, b: &str) -> usize {
    let a: Vec<char> = a.chars().collect();
    let b: Vec<char> = b.chars().collect();
    let mut d = vec![vec![0usize; b.len() + 1]; a.len() + 1];
    for (i, row) in d.iter_mut().enumerate() {
        row[0] = i;
    }
    d[0] = (0..=b.len()).collect();
    for i in 1..=a.len() {
        for j in 1..=b.len() {
            let cost = usize::from(a[i - 1] != b[j - 1]);
            let mut v = (d[i - 1][j] + 1)
                .min(d[i][j - 1] + 1)
                .min(d[i - 1][j - 1] + cost);
            if i > 1 && j > 1 && a[i - 1] == b[j - 2] && a[i - 2] == b[j - 1] {
                v = v.min(d[i - 2][j - 2] + 1);
            }
            d[i][j] = v;
        }
    }
    d[a.len()][b.len()]
}

/// Up to two of `cands` (`(folded key, payload)`) close to `key`, best first: edit distance
/// within `max(1, len/3)`, or `key` an abbreviation of the name (`cnt` for `count`: three letters
/// or more, same first letter, letters in order, name at most twice as long), ranked after
/// the edit matches. A second one comes only when it ranks equal to the first.
pub fn similar<'a, T>(key: &str, cands: impl Iterator<Item = (&'a str, T)>) -> Vec<T> {
    let limit = (key.chars().count() / 3).max(1);
    let mut found: Vec<(usize, &str, T)> = cands
        .filter(|(c, _)| *c != key)
        .filter_map(|(c, t)| {
            let d = distance(key, c);
            let rank = if d <= limit {
                d
            } else if abbreviates(key, c) {
                limit + 1
            } else {
                return None;
            };
            Some((rank, c, t))
        })
        .collect();
    found.sort_by(|a, b| (a.0, a.1).cmp(&(b.0, b.1)));
    let best = found.first().map(|f| f.0);
    found
        .into_iter()
        .take_while(|f| Some(f.0) == best)
        .take(2)
        .map(|f| f.2)
        .collect()
}

fn abbreviates(key: &str, name: &str) -> bool {
    let (k, n) = (key.chars().count(), name.chars().count());
    if k < 3 || n > 2 * k || key.chars().next() != name.chars().next() {
        return false;
    }
    let mut rest = name.chars();
    key.chars().all(|c| rest.any(|x| x == c))
}

/// The help line for the `cands` = (folded key, spelling) close to `key`, all of kind `noun`.
pub fn suggest<'a>(
    key: &str,
    noun: &'static str,
    cands: impl Iterator<Item = (&'a str, &'a str)>,
) -> Option<String> {
    let found = similar(key, cands);
    let shown: Vec<_> = found.iter().map(|&n| (n.to_string(), noun)).collect();
    (!shown.is_empty()).then(|| help(&shown))
}

/// The help line for `found` = (original spelling, kind): `a signal with a similar name exists:
/// `count``, or both when two names are equally close.
pub fn help(found: &[(String, &str)]) -> String {
    match found {
        [(n, kind)] => {
            let a = if kind.starts_with(['a', 'e', 'i', 'o', 'u']) {
                "an"
            } else {
                "a"
            };
            format!("{a} {kind} with a similar name exists: `{n}`")
        }
        _ => {
            let names: Vec<String> = found.iter().map(|(n, k)| format!("`{n}` ({k})")).collect();
            format!("similar names exist: {}", names.join(", "))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pick<'a>(key: &str, c: &[&'a str]) -> Vec<&'a str> {
        similar(key, c.iter().map(|&n| (n, n)))
    }

    #[test]
    fn similar_finds_typos_and_abbreviations() {
        assert_eq!(distance("ab", "ba"), 1);
        let c = ["numeric_std", "std_logic_1164"];
        assert_eq!(pick("numeric_sdt", &c), ["numeric_std"]);
        assert!(pick("xyz", &c).is_empty());
        assert_eq!(pick("cnt", &["count", "other"]), ["count"]);
        // too short, other first letter, or name too long: no abbreviation
        assert!(pick("ct", &["count"]).is_empty());
        assert!(pick("ont", &["count"]).is_empty());
        assert!(pick("cnt", &["counter_value"]).is_empty());
    }

    #[test]
    fn similar_prefers_edits_and_keeps_ties() {
        assert_eq!(pick("clk", &["clock", "clx"]), ["clx"]);
        assert_eq!(pick("data", &["datb", "datc", "dat"]), ["dat", "datb"]);
    }

    #[test]
    fn help_wording() {
        assert_eq!(
            help(&[("count".into(), "signal")]),
            "a signal with a similar name exists: `count`"
        );
        assert_eq!(
            help(&[("Idx".into(), "attribute")]),
            "an attribute with a similar name exists: `Idx`"
        );
        assert_eq!(
            help(&[("a".into(), "signal"), ("b".into(), "constant")]),
            "similar names exist: `a` (signal), `b` (constant)"
        );
    }
}

//! Mixed 1993/2008/2019 designs (sem.md 0.1). Under a 2019 design revision every std/ieee
//! declaration the 2008 copy lacks gets `since: V2019` (same designator and type marks of
//! the profile count as the same declaration); a 2008 file that names one gets `E1007`.
//! Under a 2008 design a std/ieee unit without a 1993 copy gets `since: V2008` (units only,
//! the items of a unit are not compared). INTEGER bounds that differ between the two copies
//! get `W1008` in a 2008 file.

use std::path::Path;

use yavs_diag::{Diagnostic, FileId, Span};
use yavs_syntax::ast;

use crate::attr::PredefAttr;
use crate::expr::names::NameRes;
use crate::ids::LibId;
use crate::ids::{SubtypeId, UnitId};
use crate::loader::UnitKey;
use crate::stat::ConstValue;
use crate::{Db, LangRev, diag};

/// Which declarations of a std/ieee unit are new in 2019.
pub(crate) enum Fresh {
    /// The 2008 tree has no such unit.
    Unit,
    /// Spans of the package items the 2008 copy lacks.
    Items(Vec<Span>),
}

impl Db {
    /// Compare std/ieee primary unit `unit` (from `path` under `<lib>/2019`) with its 2008 copy.
    pub(crate) fn fresh_in_2019(
        &mut self,
        lib: LibId,
        key: UnitKey,
        path: &Path,
        unit: &ast::DesignUnit,
        file: FileId,
    ) -> Fresh {
        let ast::LibraryUnit::Package(p) = &unit.unit else {
            return Fresh::Items(Vec::new());
        };
        let (Some(dir), Some(name)) = (path.parent().and_then(Path::parent), path.file_name())
        else {
            return Fresh::Items(Vec::new());
        };
        let old = dir.join(LangRev::Vhdl2008.as_str()).join(name);
        if !crate::stdlib::is_file(&old) {
            return Fresh::Unit;
        }
        if self
            .parse_into_cache(lib, &old, LangRev::Vhdl2008, None)
            .is_err()
        {
            return Fresh::Items(Vec::new());
        }
        let fc = &self.files[&(lib, old)];
        let old_sigs: Vec<String> = fc
            .units
            .iter()
            .flatten()
            .filter(|(k, _)| *k == key)
            .filter_map(|(_, u)| match u.as_ref().map(|u| &u.unit) {
                Some(ast::LibraryUnit::Package(p)) => Some(p),
                _ => None,
            })
            .flat_map(|p| sigs(&p.decls, self.sources.get(fc.file)))
            .map(|(s, _)| s)
            .collect();
        Fresh::Items(
            sigs(&p.decls, self.sources.get(file))
                .into_iter()
                .filter(|(s, _)| !old_sigs.contains(s))
                .map(|(_, span)| span)
                .collect(),
        )
    }

    /// Whether std/ieee primary unit `unit` has no copy in the 1993 tree (a 2008 design).
    pub(crate) fn fresh_in_2008(&self, path: &Path) -> Fresh {
        let (Some(dir), Some(name)) = (path.parent().and_then(Path::parent), path.file_name())
        else {
            return Fresh::Items(Vec::new());
        };
        if crate::stdlib::is_file(&dir.join(LangRev::Vhdl1993.as_str()).join(name)) {
            Fresh::Items(Vec::new())
        } else {
            Fresh::Unit
        }
    }

    /// Set `since: rev` on what `fresh` says is new in std/ieee unit `u`.
    pub(crate) fn mark_fresh(&mut self, u: UnitId, fresh: Fresh, rev: LangRev) {
        let spans = match fresh {
            Fresh::Unit => {
                let d = self.units[u].decl;
                self.decls[d].since = rev;
                return;
            }
            Fresh::Items(s) if s.is_empty() => return,
            Fresh::Items(s) => s,
        };
        let ds: Vec<_> = self.scopes[self.units[u].scope]
            .names
            .values()
            .flatten()
            .copied()
            .collect();
        for d in ds {
            let at = self.decls[d].span;
            if spans
                .iter()
                .any(|s| s.file == at.file && (s.start..s.end).contains(&at.start))
            {
                self.decls[d].since = rev;
            }
        }
    }

    /// The name of a revision in messages.
    pub(crate) fn rev_name(rev: LangRev) -> &'static str {
        match rev {
            LangRev::Vhdl1993 => "VHDL-1993",
            LangRev::Vhdl2008 => "VHDL-2008",
            LangRev::Vhdl2019 => "VHDL-2019",
        }
    }

    /// `E1007`: `what`, new in revision `needs`, is named at `at` in an older file.
    pub(crate) fn not_in_file_rev(
        &mut self,
        what: String,
        at: Span,
        needs: LangRev,
    ) -> crate::Reported {
        if !self.rev_reported.insert(at) {
            return crate::Reported;
        }
        let (file, need) = (Self::rev_name(self.file_rev()), Self::rev_name(needs));
        let mut d = Diagnostic::error(
            diag::NOT_IN_REVISION,
            format!("`{what}` is not available in {file}"),
        )
        .with_primary(at, format!("needs {need}"));
        if let Some(c) = self.cur() {
            let path = &self.sources.get(self.units[c.unit].file).path;
            d = d.with_note(format!("file `{path}` is analyzed as {file}"));
        }
        let design = Self::rev_name(self.rev);
        self.error(d.with_help(format!(
            "the design is elaborated with the {design} std because another unit is {design}"
        )))
    }

    /// A lookup result without the declarations newer than the file (sem.md 0.1): a 2008
    /// file sees the 2008 overloads. `E1007` at `at` when nothing is left.
    pub(crate) fn rev_gate(&mut self, r: NameRes, name: Span, at: Span) -> NameRes {
        let rev = self.file_rev();
        // No declaration is newer than the design, so nothing is hidden from a file of the
        // design's revision.
        if self.rev <= rev {
            return r;
        }
        let what = |db: &Db| db.span_text(name);
        match r {
            NameRes::One(d) if self.decls[d].since > rev => {
                self.not_in_file_rev(what(self), at, self.decls[d].since);
                NameRes::Error
            }
            NameRes::Overloads(mut ds) if ds.iter().any(|&d| self.decls[d].since > rev) => {
                let all = ds.clone();
                ds.retain(|&d| self.decls[d].since <= rev);
                if ds.is_empty() {
                    let since = self.decls[all[0]].since;
                    self.not_in_file_rev(what(self), at, since);
                    NameRes::Error
                } else {
                    NameRes::Overloads(ds)
                }
            }
            r => r,
        }
    }

    /// `W1008` for `'left`, `'right`, `'high`, `'low` of an INTEGER subtype in a 2008 file of
    /// a 2019 design when the bound lies outside the 32-bit 2008 INTEGER, once per span.
    pub(crate) fn integer_bound_changed(&mut self, a: PredefAttr, st: SubtypeId, span: Span) {
        if self.file_rev() >= LangRev::Vhdl2019
            || self.rev != LangRev::Vhdl2019
            || !matches!(
                a,
                PredefAttr::Left | PredefAttr::Right | PredefAttr::High | PredefAttr::Low
            )
            || self.well_known.integer != Some(self.base(st))
        {
            return;
        }
        let Some(ConstValue::Int(v)) = self.bound(st, a) else {
            return;
        };
        if i32::try_from(v).is_ok() || !self.rev_reported.insert(span) {
            return;
        }
        let what = self.span_text(span);
        let mut d = Diagnostic::warning(
            diag::W_CHANGED_MEANING,
            format!("`{what}` means something different under VHDL-2019"),
        )
        .with_primary(
            span,
            format!(
                "{v} here, a 32-bit bound under {}",
                Self::rev_name(self.file_rev())
            ),
        );
        if let Some(c) = self.cur() {
            let path = &self.sources.get(self.units[c.unit].file).path;
            d = d.with_note(format!(
                "file `{path}` is analyzed as {}",
                Self::rev_name(self.file_rev())
            ));
        }
        self.report(d.with_note(
            "the design is elaborated with the VHDL-2019 std, where INTEGER is 64-bit, because another unit is VHDL-2019",
        ));
    }

    pub(crate) fn span_text(&self, s: Span) -> String {
        let f = self.sources.get(s.file);
        f.text()[f.text_offset(s.start)..f.text_offset(s.end)].to_string()
    }
}

/// One signature per package item that declares something: kind, designator and, for
/// subprograms, the type marks of the profile (lower case, without blanks).
fn sigs(decls: &[ast::Decl], src: &yavs_diag::SourceFile) -> Vec<(String, Span)> {
    let text = |s: Span| -> String {
        src.text()[src.text_offset(s.start)..src.text_offset(s.end)]
            .chars()
            .filter(|c| !c.is_whitespace())
            .flat_map(char::to_lowercase)
            .collect()
    };
    let des = |d: &ast::Designator| match d {
        ast::Designator::Ident(i) => i.key.to_string(),
        ast::Designator::Operator(op, _) => format!("\"{}\"", op.as_str().to_ascii_lowercase()),
        ast::Designator::Char(c) => format!("{c:?}"),
    };
    let mut out = Vec::new();
    let mut one = |kind: &str, name: String, span: Span| out.push((format!("{kind} {name}"), span));
    for d in decls {
        match d {
            ast::Decl::Type(t) => one("type", t.name.key.to_string(), t.span),
            ast::Decl::Subtype(t) => one("subtype", t.name.key.to_string(), t.span),
            ast::Decl::Object(o) => o
                .names
                .iter()
                .for_each(|n| one("object", n.key.to_string(), o.span)),
            ast::Decl::File(o) => o
                .names
                .iter()
                .for_each(|n| one("file", n.key.to_string(), o.span)),
            ast::Decl::Alias(a) => one("alias", des(&a.designator), a.span),
            ast::Decl::Component(c) => one("component", c.name.key.to_string(), c.span),
            ast::Decl::Attribute(a) => one("attribute", a.name.key.to_string(), a.span),
            ast::Decl::Subprogram(s) => {
                let params: Vec<String> = s
                    .params
                    .iter()
                    .flatten()
                    .flat_map(|p| match p {
                        ast::InterfaceDecl::Object(o) => {
                            let t = match &o.subtype {
                                Some(ast::IncompleteSubtype::Subtype(si)) => {
                                    text(si.type_mark.span)
                                }
                                _ => "?".to_string(),
                            };
                            vec![t; o.names.len()]
                        }
                        _ => vec!["?".to_string()],
                    })
                    .collect();
                let ret = s.return_type.as_ref().map(|n| text(n.span));
                one(
                    "subprogram",
                    format!(
                        "{}({}){}",
                        des(&s.designator),
                        params.join(","),
                        ret.unwrap_or_default()
                    ),
                    s.span,
                );
            }
            ast::Decl::SubprogramInstance(s) => one("instance", des(&s.designator), s.span),
            ast::Decl::Package(p) => one("package", p.name.key.to_string(), p.span),
            ast::Decl::PackageInstance(p) => one("package", p.name.key.to_string(), p.span),
            ast::Decl::View(v) => one("view", v.name.key.to_string(), v.span),
            _ => {}
        }
    }
    out
}

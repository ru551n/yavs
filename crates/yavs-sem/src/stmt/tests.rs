//! Tests of statement analysis (stmt.rs) and of prefixes, drivers and sources
//! (lsp.rs). Statements without expressions go through the parser; the rest
//! is built as HIR directly, since expression resolution is another module's.

use yavs_diag::{FileId, Span};
use yavs_syntax::Symbol;
use yavs_syntax::ast::{self, Ident};

use super::*;
use crate::Options;
use crate::decl::{Body, Param, Subprogram};
use crate::hir::LspStep;
use crate::hir::{Actual, AggAssoc, AggBounds, Arg, Expr, GenAlt, ProcessInfo, WaveElem};
use crate::lsp::Overlap;
use crate::stat::{ConstValue, Staticness};
use crate::types::{Constraint, Direction, RecordElem, Resolution, SubtypeDef, TypeDef, TypeKind};

fn ident(s: &str) -> Ident {
    Ident {
        text: Symbol::intern(s),
        key: Symbol::intern(&s.to_ascii_lowercase()),
        span: Span::default(),
    }
}

fn new_db() -> Db {
    Db::new(Options::new(LangRev::Vhdl2008, crate::default_stdlib()))
}

/// Error codes with 1-based `(line, col)` of the primary span.
fn codes(db: &Db, file: FileId) -> Vec<(&'static str, usize, usize)> {
    db.diagnostics()
        .iter()
        .map(|d| {
            let (l, c) = d
                .primary
                .as_ref()
                .map_or((0, 0), |p| db.sources.get(file).line_col(p.span.start));
            (d.code, l, c)
        })
        .collect()
}

// -- parser-driven: statements without expressions ---------------------------

fn parse(db: &mut Db, src: &str, std: LangRev) -> (FileId, ast::DesignFile) {
    let file = db.sources.add("t.vhd", src);
    let p = yavs_syntax::parse_file(
        file,
        db.sources.get(file),
        &yavs_syntax::Options {
            std,
            defines: Vec::new(),
            psl: false,
        },
    );
    assert!(p.diags.is_empty(), "{:?}", p.diags);
    (file, p.design.expect("parses"))
}

/// Analyze the statements of the first architecture in `src`.
fn arch(src: &str) -> (Db, FileId, Result<Box<[StmtId]>, Reported>) {
    arch_std(src, LangRev::Vhdl2008)
}

fn arch_std(src: &str, std: LangRev) -> (Db, FileId, Result<Box<[StmtId]>, Reported>) {
    let mut db = new_db();
    let (file, df) = parse(&mut db, src, std);
    let a = df
        .units
        .iter()
        .find_map(|u| match &u.unit {
            ast::LibraryUnit::Architecture(a) => Some(a),
            _ => None,
        })
        .expect("an architecture");
    let scope = db.add_scope(ScopeKind::Architecture, None, None);
    let r = db.analyze_conc_stmts(scope, &a.stmts, false);
    (db, file, r)
}

/// Analyze the body of the first subprogram body in a package body.
fn subprogram(src: &str) -> (Db, FileId, Result<Box<[StmtId]>, Reported>) {
    let mut db = new_db();
    let (file, df) = parse(&mut db, src, LangRev::Vhdl2008);
    let body = df
        .units
        .iter()
        .find_map(|u| match &u.unit {
            ast::LibraryUnit::PackageBody(b) => b.decls.iter().find_map(|d| match d {
                ast::Decl::SubprogramBody(b) => Some(b),
                _ => None,
            }),
            _ => None,
        })
        .expect("a subprogram body");
    let kind = match body.spec.kind {
        ast::SubprogramKind::Procedure => SubprogramKind::Procedure,
        ast::SubprogramKind::Function(p) => SubprogramKind::Function {
            pure: p != Some(ast::Purity::Impure),
        },
    };
    let (decl, scope) = add_subprogram(&mut db, "f", kind);
    let r = db.analyze_subprogram_stmts(decl, scope, &body.stmts);
    (db, file, r)
}

fn add_subprogram(db: &mut Db, name: &str, kind: SubprogramKind) -> (DeclId, ScopeId) {
    let outer = db.add_scope(ScopeKind::Architecture, None, None);
    let scope = db.add_scope(ScopeKind::Subprogram, Some(outer), None);
    let ret = matches!(kind, SubprogramKind::Function { .. }).then_some(db.poison.subtype);
    let decl = db.decls.push(Decl {
        name: Designator::Ident(ident(name)),
        kind: DeclKind::Subprogram(Subprogram {
            kind,
            params: Box::new([]),
            ret,
            generics: None,
            scope: Some(scope),
            body: Body::Pending,
            mangled: Symbol::intern(name),
            ret_mark: None,
            impure_reads: false,
        }),
        scope: outer,
        span: Span::default(),
        implicit: false,
        since: LangRev::Vhdl1993,
    });
    (decl, scope)
}

const ENT: &str = "entity e is end entity;\n";

#[test]
fn wait_in_process_all_is_e1801() {
    let (db, f, r) = arch(&format!(
        "{ENT}architecture a of e is\nbegin\n  process (all) begin\n    wait;\n  end process;\nend architecture;\n"
    ));
    assert!(r.is_err());
    assert_eq!(codes(&db, f), [(diag::BAD_WAIT, 5, 5)]);
    assert_eq!(db.diagnostics()[0].secondary.len(), 1, "points at the list");
}

#[test]
fn wait_in_plain_process_is_fine() {
    let (db, _, r) = arch(&format!(
        "{ENT}architecture a of e is\nbegin\n  p: process begin\n    wait;\n  end process;\nend architecture;\n"
    ));
    assert!(r.is_ok(), "{:?}", db.diagnostics());
    let id = r.unwrap()[0];
    let StmtKind::Process {
        sensitivity, body, ..
    } = &db.stmts[id].kind
    else {
        panic!("process")
    };
    assert!(matches!(sensitivity, Sensitivity::None));
    assert!(matches!(
        &db.stmts[body[0]].kind,
        StmtKind::Wait { on, until: None, timeout: None } if on.is_empty()
    ));
    assert_eq!(db.stmts[id].label.map(|l| l.key), Some(Symbol::intern("p")));
}

#[test]
fn exit_and_next_outside_loop_are_e1811() {
    let (db, f, r) = arch(&format!(
        "{ENT}architecture a of e is\nbegin\n  process begin\n    exit;\n    next;\n    wait;\n  end process;\nend architecture;\n"
    ));
    assert!(r.is_err());
    // Both reported: analysis continues after an error.
    assert_eq!(
        codes(&db, f),
        [
            (diag::NEXT_OUTSIDE_LOOP, 5, 5),
            (diag::NEXT_OUTSIDE_LOOP, 6, 5)
        ]
    );
}

#[test]
fn exit_unknown_label_points_at_label() {
    let (db, f, r) = arch(&format!(
        "{ENT}architecture a of e is\nbegin\n  process begin\n    l1: loop\n      exit lp;\n    end loop;\n  end process;\nend architecture;\n"
    ));
    assert!(r.is_err());
    assert_eq!(codes(&db, f), [(diag::NEXT_OUTSIDE_LOOP, 6, 12)]);
}

#[test]
fn next_and_exit_bind_to_the_named_loop() {
    let (db, _, r) = arch(&format!(
        "{ENT}architecture a of e is\nbegin\n  process begin\n    outer: loop\n      inner: loop\n        next outer;\n        exit;\n      end loop;\n    end loop;\n  end process;\nend architecture;\n"
    ));
    let ids = r.unwrap_or_else(|_| panic!("{:?}", db.diagnostics()));
    let StmtKind::Process { body, .. } = &db.stmts[ids[0]].kind else {
        panic!()
    };
    let outer = body[0];
    let StmtKind::Loop {
        body: ob, scope, ..
    } = &db.stmts[outer].kind
    else {
        panic!()
    };
    // The loop region is owned by its label (expanded names `outer.x`).
    let owner = db.scopes[*scope].owner.expect("owned by the label");
    assert!(matches!(db.decls[owner].kind, DeclKind::Label(s) if s == outer));
    let inner = ob[0];
    let StmtKind::Loop { body: ib, .. } = &db.stmts[inner].kind else {
        panic!()
    };
    assert!(matches!(db.stmts[ib[0]].kind, StmtKind::Next { loop_, .. } if loop_ == outer));
    assert!(matches!(db.stmts[ib[1]].kind, StmtKind::Exit { loop_, .. } if loop_ == inner));
}

#[test]
fn return_in_process_is_e1802() {
    let (db, f, r) = arch(&format!(
        "{ENT}architecture a of e is\nbegin\n  process begin\n    return;\n  end process;\nend architecture;\n"
    ));
    assert!(r.is_err());
    assert_eq!(codes(&db, f), [(diag::BAD_RETURN, 5, 5)]);
}

#[test]
fn duplicate_labels_are_e1101() {
    let (db, f, r) = arch(&format!(
        "{ENT}architecture a of e is\nbegin\n  process begin\n    l: null;\n    l: null;\n    wait;\n  end process;\nend architecture;\n"
    ));
    assert!(r.is_err());
    assert_eq!(codes(&db, f), [(diag::REDECLARED, 6, 5)]);
}

#[test]
fn sequential_block_2019() {
    let (db, _, r) = arch_std(
        &format!(
            "{ENT}architecture a of e is\nbegin\n  process begin\n    b: block is begin\n      null;\n    end block;\n    wait;\n  end process;\nend architecture;\n"
        ),
        LangRev::Vhdl2019,
    );
    let ids = r.unwrap_or_else(|_| panic!("{:?}", db.diagnostics()));
    let StmtKind::Process { body, .. } = &db.stmts[ids[0]].kind else {
        panic!()
    };
    let StmtKind::SeqBlock { scope, body: bb } = &db.stmts[body[0]].kind else {
        panic!("seq block")
    };
    assert_eq!(db.scopes[*scope].kind, ScopeKind::SeqBlock);
    assert!(matches!(db.stmts[bb[0]].kind, StmtKind::Null));
}

#[test]
fn unsupported_pieces_say_so() {
    // A construct analysis does not implement: E1999 naming the slice, never silence.
    let (db, _, r) = arch(&format!(
        "{ENT}architecture a of e is\nbegin\n  process begin\n    wait for 1 ns;\n  end process;\nend architecture;\n"
    ));
    assert!(r.is_err());
    assert_eq!(db.diagnostics()[0].code, diag::INTERNAL);
}

const PKG: &str = "package p is\nend package;\npackage body p is\n";

#[test]
fn function_rules() {
    let (db, f, r) = subprogram(&format!(
        "{PKG}  function f return integer is\n  begin\n    wait;\n    return;\n  end function;\nend package body;\n"
    ));
    assert!(r.is_err());
    assert_eq!(
        codes(&db, f),
        [(diag::BAD_WAIT, 6, 5), (diag::BAD_RETURN, 7, 5)]
    );
    assert_eq!(
        db.diagnostics()[0].message,
        "`wait` is not allowed in a function"
    );
    assert_eq!(
        db.diagnostics()[1].message,
        "a function return needs a value"
    );
}

#[test]
fn procedure_rules() {
    let (db, _, r) = subprogram(&format!(
        "{PKG}  procedure f is\n  begin\n    wait;\n    return;\n  end procedure;\nend package body;\n"
    ));
    assert!(r.is_ok(), "{:?}", db.diagnostics());
}

#[test]
fn procedure_inside_sensitized_process_or_function_cannot_wait() {
    let mut db = new_db();
    let src =
        format!("{PKG}  procedure f is\n  begin\n    wait;\n  end procedure;\nend package body;\n");
    let (file, df) = parse(&mut db, &src, LangRev::Vhdl2008);
    let ast::LibraryUnit::PackageBody(b) = &df.units[1].unit else {
        panic!()
    };
    let ast::Decl::SubprogramBody(body) = &b.decls[0] else {
        panic!()
    };
    let (decl, scope) = add_subprogram(&mut db, "f", SubprogramKind::Procedure);
    // As if decl.rs analyzed it while a sensitized process was open.
    db.stmt_owners.push(Owner::Process {
        sensitivity: Some(Span::new(file, 0, 1)),
    });
    assert!(
        db.analyze_subprogram_stmts(decl, scope, &body.stmts)
            .is_err()
    );
    db.stmt_owners.pop();
    // And inside a function.
    let (fdecl, _) = add_subprogram(&mut db, "g", SubprogramKind::Function { pure: true });
    db.stmt_owners.push(Owner::Subprogram {
        decl: fdecl,
        kind: SubprogramKind::Function { pure: true },
        in_process: false,
        wait_ban: Some(WaitBan::Function { own: true }),
    });
    let (decl, scope) = add_subprogram(&mut db, "f2", SubprogramKind::Procedure);
    assert!(
        db.analyze_subprogram_stmts(decl, scope, &body.stmts)
            .is_err()
    );
    let msgs: Vec<_> = db
        .diagnostics()
        .iter()
        .map(|d| d.message.as_str())
        .collect();
    assert_eq!(
        msgs,
        [
            "`wait` is not allowed in a procedure declared in a process with a sensitivity list",
            "`wait` is not allowed in a procedure declared inside a function",
        ]
    );
}

// -- HIR-built: prefixes, drivers, targets, purity ----------------------------

struct Fx {
    db: Db,
    scope: ScopeId,
    bit: SubtypeId,
    rbit: SubtypeId,
    bv: SubtypeId,
    rec: SubtypeId,
    int: SubtypeId,
    pos: u32,
}

impl Fx {
    fn new() -> Fx {
        let mut db = new_db();
        let scope = db.add_scope(ScopeKind::Architecture, None, None);
        let ty = |db: &mut Db, name: &str, kind: TypeKind| {
            let t = db.types.next_id();
            let st = db.subtypes.push(SubtypeDef {
                base: t,
                name: Some(ident(name)),
                parent: None,
                constraint: Constraint::None,
                resolution: None,
                span: Span::default(),
            });
            db.types.push(TypeDef {
                decl: db.poison.decl,
                name: ident(name),
                anonymous: false,
                kind,
                first_subtype: st,
                base_subtype: st,
            });
            st
        };
        let bit = ty(&mut db, "bit", TypeKind::Enum { lits: Box::new([]) });
        let int = ty(&mut db, "integer", TypeKind::Integer);
        let bv = ty(
            &mut db,
            "bit_vector",
            TypeKind::Array {
                indexes: Box::new([int]),
                elem: bit,
            },
        );
        let rbit = db.subtypes.push(SubtypeDef {
            base: db.base(bit),
            name: Some(ident("rbit")),
            parent: Some(bit),
            constraint: Constraint::None,
            resolution: Some(Resolution::Function(db.poison.decl)),
            span: Span::default(),
        });
        let rec = ty(
            &mut db,
            "r",
            TypeKind::Record {
                elems: Box::new([
                    RecordElem {
                        name: ident("a"),
                        subtype: bit,
                    },
                    RecordElem {
                        name: ident("b"),
                        subtype: rbit,
                    },
                ]),
            },
        );
        Fx {
            db,
            scope,
            bit,
            rbit,
            bv,
            rec,
            int,
            pos: 0,
        }
    }

    fn span(&mut self) -> Span {
        self.pos += 10;
        Span::new(FileId(0), self.pos, self.pos + 1)
    }

    fn object(
        &mut self,
        name: &str,
        class: ObjClass,
        role: ObjRole,
        mode: Option<Mode>,
        st: SubtypeId,
    ) -> DeclId {
        let span = self.span();
        self.db.decls.push(Decl {
            name: Designator::Ident(ident(name)),
            kind: DeclKind::Object(Object {
                class,
                role,
                mode,
                subtype: st,
                default: None,
                bounds: BoundsFrom::Subtype,
                signal_kind: None,
                file_open: None,
            }),
            scope: self.scope,
            span,
            implicit: false,
            since: LangRev::Vhdl1993,
        })
    }

    fn sig(&mut self, name: &str, st: SubtypeId) -> DeclId {
        self.object(name, ObjClass::Signal, ObjRole::Plain, None, st)
    }

    fn expr(&mut self, kind: ExprKind, st: SubtypeId, stat: Staticness) -> ExprId {
        let span = self.span();
        self.db.add_expr(Expr {
            kind,
            subtype: st,
            span,
            stat,
            value: None,
        })
    }

    fn obj(&mut self, d: DeclId) -> ExprId {
        let st = match &self.db.decls[d].kind {
            DeclKind::Object(o) => o.subtype,
            _ => self.db.poison.subtype,
        };
        self.expr(ExprKind::Obj(d), st, Staticness::None)
    }

    fn int(&mut self, v: i64) -> ExprId {
        let c = self.db.add_const(ConstValue::Int(v));
        let int = self.int;
        let e = self.expr(ExprKind::Const(c), int, Staticness::Locally);
        self.db.exprs[e].value = Some(c);
        e
    }

    /// An integer expression of the given staticness without a folded value.
    fn opaque(&mut self, stat: Staticness) -> ExprId {
        let int = self.int;
        let k = self.db.add_const(ConstValue::Int(0));
        self.expr(ExprKind::Const(k), int, stat)
    }

    fn index(&mut self, p: ExprId, i: ExprId) -> ExprId {
        let bit = self.bit;
        self.expr(
            ExprKind::Index {
                prefix: p,
                indices: Box::new([i]),
            },
            bit,
            Staticness::None,
        )
    }

    fn slice(&mut self, p: ExprId, l: i64, r: i64) -> ExprId {
        let (left, right) = (self.int(l), self.int(r));
        let bv = self.bv;
        self.expr(
            ExprKind::Slice {
                prefix: p,
                range: RangeC::Explicit {
                    left,
                    dir: Direction::To,
                    right,
                },
            },
            bv,
            Staticness::None,
        )
    }

    fn field(&mut self, p: ExprId, elem: u32) -> ExprId {
        let bit = self.bit;
        self.expr(ExprKind::Field { prefix: p, elem }, bit, Staticness::None)
    }

    fn assign(&mut self, target: ExprId) -> StmtId {
        let lsps: Box<[Lsp]> = self.db.lsp(target).into_iter().collect();
        let v = self.int(0);
        let span = self.span();
        self.db.stmts.push(Stmt {
            id: self.db.stmts.next_id(),
            label: None,
            span,
            kind: StmtKind::SignalAssign {
                target: Target { expr: target, lsps },
                delay: Delay::Default,
                rhs: Rhs::Simple(Waveform::Elements(Box::new([WaveElem {
                    value: Some(v),
                    after: None,
                }]))),
            },
        })
    }

    fn stmt(&mut self, kind: StmtKind) -> StmtId {
        let span = self.span();
        self.db.stmts.push(Stmt {
            id: self.db.stmts.next_id(),
            label: None,
            span,
            kind,
        })
    }

    fn process(&mut self, body: Vec<StmtId>) -> StmtId {
        let scope = self
            .db
            .add_scope(ScopeKind::Process, Some(self.scope), None);
        self.process_in(scope, body, false)
    }

    fn process_in(&mut self, scope: ScopeId, body: Vec<StmtId>, all: bool) -> StmtId {
        let info = self.db.process_info(scope, &body, all);
        self.stmt(StmtKind::Process {
            postponed: false,
            sensitivity: if all {
                Sensitivity::All
            } else {
                Sensitivity::None
            },
            info,
            scope,
            body: body.into(),
        })
    }

    fn sources_error(&mut self, top: &[StmtId]) -> Vec<&'static str> {
        self.db.check_multiple_sources(top);
        self.db.diagnostics().iter().map(|d| d.code).collect()
    }
}

#[test]
fn lsp_keeps_static_indices_and_cuts_at_non_static() {
    let mut fx = Fx::new();
    let bv = fx.bv;
    let s = fx.sig("s", bv);
    let so = fx.obj(s);
    let three = fx.int(3);
    let s3 = fx.index(so, three);
    let l = fx.db.lsp(s3).unwrap();
    assert_eq!(l.root, ObjRoot::Decl(s));
    assert!(matches!(&l.path[..], [LspStep::Index(_)]));
    assert_eq!(l.stat, Staticness::Locally);

    // A loop parameter index is not static: the prefix is the whole signal.
    let i = fx.opaque(Staticness::None);
    let so = fx.obj(s);
    let si = fx.index(so, i);
    let l = fx.db.lsp(si).unwrap();
    assert!(l.path.is_empty());

    // A generate parameter (globally static) stays in the prefix (sim1 #856).
    let g = fx.opaque(Staticness::Globally);
    let so = fx.obj(s);
    let sg = fx.index(so, g);
    let l = fx.db.lsp(sg).unwrap();
    assert!(matches!(&l.path[..], [LspStep::Index(_)]));
    assert_eq!(l.stat, Staticness::Globally);

    // `s(1 to 2)(2)` is the element `s(2)`.
    let so = fx.obj(s);
    let sl = fx.slice(so, 1, 2);
    let two = fx.int(2);
    let e = fx.index(sl, two);
    let l = fx.db.lsp(e).unwrap();
    assert!(matches!(&l.path[..], [LspStep::Index(_)]));
}

#[test]
fn lsp_follows_object_aliases_and_fields() {
    let mut fx = Fx::new();
    let rec = fx.rec;
    let s = fx.sig("s", rec);
    let so = fx.obj(s);
    let sa = fx.field(so, 0);
    let al = fx.db.decls.push(Decl {
        name: Designator::Ident(ident("al")),
        kind: DeclKind::Alias(crate::decl::Alias::Object {
            target: sa,
            subtype: None,
        }),
        scope: fx.scope,
        span: Span::default(),
        implicit: false,
        since: LangRev::Vhdl1993,
    });
    let ae = fx.obj(al);
    let l = fx.db.lsp(ae).unwrap();
    assert_eq!(l.root, ObjRoot::Decl(s));
    assert!(matches!(&l.path[..], [LspStep::Field(0)]));
    assert_eq!(fx.db.name_root(ae), Some(NameRoot::Obj(ObjRoot::Decl(s))));
}

#[test]
fn overlap_of_prefixes() {
    let mut fx = Fx::new();
    let bv = fx.bv;
    let s = fx.sig("s", bv);
    let at = |fx: &mut Fx, i: i64| {
        let so = fx.obj(s);
        let ie = fx.int(i);
        let e = fx.index(so, ie);
        fx.db.lsp(e).unwrap()
    };
    let (s0, s1, s2) = (at(&mut fx, 0), at(&mut fx, 1), at(&mut fx, 2));
    let so = fx.obj(s);
    let whole = fx.db.lsp(so).unwrap();
    let so = fx.obj(s);
    let e = fx.slice(so, 1, 2);
    let s12 = fx.db.lsp(e).unwrap();
    let g = fx.opaque(Staticness::Globally);
    let so = fx.obj(s);
    let e = fx.index(so, g);
    let sg = fx.db.lsp(e).unwrap();
    let db = &fx.db;
    assert_eq!(db.overlap(&s0, &s1), Overlap::No);
    assert_eq!(db.overlap(&s12, &s2), Overlap::Yes);
    assert_eq!(db.overlap(&s12, &s0), Overlap::No);
    assert_eq!(db.overlap(&whole, &s1), Overlap::Yes);
    assert_eq!(db.overlap(&sg, &s1), Overlap::Maybe);
}

/// The sim1/sim2 cross-check cases a1 to a7 (crate notes in lsp.rs): sim2
/// rejects these at analysis, sim1 at elaboration with the same verdicts.
#[test]
fn multiple_sources_cases() {
    // a1: s(0) and s(1) from two processes: fine.
    let mut fx = Fx::new();
    let bv = fx.bv;
    let s = fx.sig("s", bv);
    let el = |fx: &mut Fx, i: i64| {
        let so = fx.obj(s);
        let ie = fx.int(i);
        fx.index(so, ie)
    };
    let t0 = el(&mut fx, 0);
    let a0 = fx.assign(t0);
    let t1 = el(&mut fx, 1);
    let a1 = fx.assign(t1);
    let p1 = fx.process(vec![a0]);
    let p2 = fx.process(vec![a1]);
    assert!(fx.sources_error(&[p1, p2]).is_empty());

    // a2: s(i) in a loop (whole `s`) and s(3): error, both spans.
    let i = fx.opaque(Staticness::None);
    let so = fx.obj(s);
    let ti = fx.index(so, i);
    let ai = fx.assign(ti);
    let lscope = fx.db.add_scope(ScopeKind::Loop, Some(fx.scope), None);
    let lp = fx.stmt(StmtKind::Loop {
        scheme: LoopScheme::Forever,
        scope: lscope,
        body: Box::new([ai]),
    });
    let t3 = el(&mut fx, 3);
    let a3 = fx.assign(t3);
    let p1 = fx.process(vec![lp]);
    let p2 = fx.process(vec![a3]);
    assert_eq!(fx.sources_error(&[p1, p2]), [diag::MULTIPLE_SOURCES]);
    let d = &fx.db.diagnostics()[0];
    assert_eq!(d.secondary.len(), 1);
    assert_eq!(d.primary.as_ref().unwrap().span, fx.db.exprs[t3].span);
    assert_eq!(d.secondary[0].span, fx.db.exprs[ti].span);
}

#[test]
fn multiple_sources_records_generates_and_resolution() {
    let mut fx = Fx::new();
    let (rec, rbit, bv) = (fx.rec, fx.rbit, fx.bv);
    let s = fx.sig("s", rec);
    let fld = |fx: &mut Fx, i: u32| {
        let so = fx.obj(s);
        let e = fx.field(so, i);
        let a = fx.assign(e);
        // A concurrent assignment is its own source.
        let StmtKind::SignalAssign { target, delay, rhs } =
            std::mem::replace(&mut fx.db.stmts[a].kind, StmtKind::Null)
        else {
            unreachable!()
        };
        fx.db.stmts[a].kind = StmtKind::ConcSignalAssign {
            postponed: false,
            guarded: false,
            guard: None,
            target,
            delay,
            rhs,
            info: ProcessInfo::default(),
        };
        let info = fx.db.conc_info(a, None);
        if let StmtKind::ConcSignalAssign { info: i, .. } = &mut fx.db.stmts[a].kind {
            *i = info;
        }
        a
    };
    // a5: s.a twice is an error; s.b is resolved, twice is fine.
    let (x, y) = (fld(&mut fx, 1), fld(&mut fx, 1));
    assert!(fx.sources_error(&[x, y]).is_empty());
    let (x, y, z) = (fld(&mut fx, 0), fld(&mut fx, 1), fld(&mut fx, 0));
    assert_eq!(fx.sources_error(&[x, y, z]), [diag::MULTIPLE_SOURCES]);

    // a6: sources inside generates are left to elab (conditional or repeated).
    let mut fx = Fx::new();
    let bit = fx.bit;
    let s = fx.sig("s", bit);
    let alt = |fx: &mut Fx| {
        let so = fx.obj(s);
        let a = fx.assign(so);
        let p = fx.process(vec![a]);
        let scope = fx.db.add_scope(ScopeKind::Generate, Some(fx.scope), None);
        GenAlt {
            label: None,
            cond: None,
            scope,
            body: Box::new([p]),
        }
    };
    let (g1, g2) = (alt(&mut fx), alt(&mut fx));
    let g = fx.stmt(StmtKind::IfGenerate {
        alts: Box::new([g1, g2]),
    });
    assert!(fx.sources_error(&[g]).is_empty());
    // Even against a process outside: `if C generate` may not exist.
    let so = fx.obj(s);
    let a = fx.assign(so);
    let p = fx.process(vec![a]);
    assert!(fx.sources_error(&[g, p]).is_empty());

    // A resolved element type: never an error.
    let mut fx = Fx::new();
    let rv = fx.db.subtypes.push(SubtypeDef {
        base: fx.db.base(bv),
        name: None,
        parent: None,
        constraint: Constraint::None,
        resolution: None,
        span: Span::default(),
    });
    let _ = rv;
    let rbv_t = fx.db.types.next_id();
    let rbv = fx.db.subtypes.push(SubtypeDef {
        base: rbv_t,
        name: None,
        parent: None,
        constraint: Constraint::None,
        resolution: None,
        span: Span::default(),
    });
    let int = fx.int;
    fx.db.types.push(TypeDef {
        decl: fx.db.poison.decl,
        name: ident("std_logic_vector"),
        anonymous: false,
        kind: TypeKind::Array {
            indexes: Box::new([int]),
            elem: rbit,
        },
        first_subtype: rbv,
        base_subtype: rbv,
    });
    let s = fx.sig("s", rbv);
    let so = fx.obj(s);
    let a = fx.assign(so);
    let so = fx.obj(s);
    let b = fx.assign(so);
    let (p1, p2) = (fx.process(vec![a]), fx.process(vec![b]));
    assert!(fx.sources_error(&[p1, p2]).is_empty());
}

#[test]
fn for_generate_sources_are_left_to_elab() {
    // a3: s(i) in a for generate, s(3) outside: elab decides.
    let mut fx = Fx::new();
    let bv = fx.bv;
    let s = fx.sig("s", bv);
    let g = fx.opaque(Staticness::Globally);
    let so = fx.obj(s);
    let t = fx.index(so, g);
    let a = fx.assign(t);
    let p = fx.process(vec![a]);
    let gscope = fx.db.add_scope(ScopeKind::Generate, Some(fx.scope), None);
    let int = fx.int;
    let param = fx.object("i", ObjClass::Constant, ObjRole::GenerateParam, None, int);
    let (l, r) = (fx.int(0), fx.int(1));
    let fg = fx.stmt(StmtKind::ForGenerate {
        param,
        range: RangeC::Explicit {
            left: l,
            dir: Direction::To,
            right: r,
        },
        scope: gscope,
        body: Box::new([p]),
    });
    let so = fx.obj(s);
    let three = fx.int(3);
    let t3 = fx.index(so, three);
    let a3 = fx.assign(t3);
    let p3 = fx.process(vec![a3]);
    assert!(fx.sources_error(&[fg, p3]).is_empty());
}

#[test]
fn instance_out_ports_are_sources() {
    let mut fx = Fx::new();
    let bit = fx.bit;
    let s = fx.sig("s", bit);
    let port = fx.object("o", ObjClass::Signal, ObjRole::Port, Some(Mode::Out), bit);
    let inport = fx.object("i", ObjClass::Signal, ObjRole::Port, Some(Mode::In), bit);
    let cscope = fx.db.add_scope(ScopeKind::Component, Some(fx.scope), None);
    let comp = fx.db.decls.push(Decl {
        name: Designator::Ident(ident("c")),
        kind: DeclKind::Component(crate::decl::Component {
            scope: cscope,
            generics: Box::new([]),
            ports: Box::new([port, inport]),
        }),
        scope: fx.scope,
        span: Span::default(),
        implicit: false,
        since: LangRev::Vhdl1993,
    });
    let inst = |fx: &mut Fx, formal: u16| {
        let so = fx.obj(s);
        fx.stmt(StmtKind::Instance {
            target: InstTarget::Component(comp),
            scope: fx.db.root,
            generic_map: Box::new([]),
            port_map: Box::new([Arg {
                formal,
                actual: Actual::Expr(so),
                inertial: false,
            }]),
        })
    };
    // Two `in` actuals read `s`: fine. Two `out` actuals drive it: error.
    let (a, b) = (inst(&mut fx, 1), inst(&mut fx, 1));
    assert!(fx.sources_error(&[a, b]).is_empty());
    let (a, b) = (inst(&mut fx, 0), inst(&mut fx, 0));
    assert_eq!(fx.sources_error(&[a, b]), [diag::MULTIPLE_SOURCES]);
}

#[test]
fn process_info_drivers_and_all() {
    let mut fx = Fx::new();
    let (bit, bv) = (fx.bit, fx.bv);
    let s = fx.sig("s", bit);
    let t = fx.sig("t", bv);
    let v = fx.object("v", ObjClass::Variable, ObjRole::Plain, None, bit);
    let u = fx.sig("u", bit);
    // t(0) <= s; t(0) <= s; t(1) <= v;  (u is driven by a local procedure)
    let tel = |fx: &mut Fx, i: i64| {
        let to = fx.obj(t);
        let ie = fx.int(i);
        fx.index(to, ie)
    };
    let mut assigns = Vec::new();
    for i in [0, 0, 1] {
        let te = tel(&mut fx, i);
        let lsps: Box<[Lsp]> = fx.db.lsp(te).into_iter().collect();
        let val = if i == 1 { fx.obj(v) } else { fx.obj(s) };
        assigns.push(fx.stmt(StmtKind::SignalAssign {
            target: Target { expr: te, lsps },
            delay: Delay::Default,
            rhs: Rhs::Simple(Waveform::Elements(Box::new([WaveElem {
                value: Some(val),
                after: None,
            }]))),
        }));
    }
    let pscope = fx.db.add_scope(ScopeKind::Process, Some(fx.scope), None);
    // procedure q is begin u <= '0'; end;  declared in the process.
    let uo = fx.obj(u);
    let qa = fx.assign(uo);
    let qscope = fx.db.add_scope(ScopeKind::Subprogram, Some(pscope), None);
    let q = fx.db.decls.push(Decl {
        name: Designator::Ident(ident("q")),
        kind: DeclKind::Subprogram(Subprogram {
            kind: SubprogramKind::Procedure,
            params: Box::new([]),
            ret: None,
            generics: None,
            scope: Some(qscope),
            body: Body::Hir {
                scope: qscope,
                stmts: Box::new([qa]),
            },
            mangled: Symbol::intern("q"),
            ret_mark: None,
            impure_reads: false,
        }),
        scope: pscope,
        span: Span::default(),
        implicit: false,
        since: LangRev::Vhdl1993,
    });
    fx.db.scopes[pscope].order.push(q);
    let p = fx.process_in(pscope, assigns, true);
    let StmtKind::Process { info, .. } = &fx.db.stmts[p].kind else {
        panic!()
    };
    let roots: Vec<_> = info
        .drivers
        .iter()
        .map(|l| (l.root, l.path.len()))
        .collect();
    assert_eq!(
        roots,
        [
            (ObjRoot::Decl(t), 1),
            (ObjRoot::Decl(t), 1),
            (ObjRoot::Decl(u), 0)
        ],
        "t(0) once, t(1), u from the local procedure"
    );
    let reads: Vec<_> = info
        .reads_all
        .as_ref()
        .unwrap()
        .iter()
        .map(|l| l.root)
        .collect();
    assert_eq!(
        reads,
        [ObjRoot::Decl(s)],
        "variables are not in the `all` set"
    );
}

#[test]
fn procedure_call_actuals_drive_and_read() {
    let mut fx = Fx::new();
    let bit = fx.bit;
    let (a, b) = (fx.sig("a", bit), fx.sig("b", bit));
    let param = |_fx: &mut Fx, mode| Param {
        name: ident("x"),
        class: ObjClass::Signal,
        mode,
        subtype: bit,
        default: None,
        decl: None,
        mark: None,
    };
    let params = Box::new([param(&mut fx, Mode::In), param(&mut fx, Mode::Out)]);
    let pr = fx.db.decls.push(Decl {
        name: Designator::Ident(ident("pr")),
        kind: DeclKind::Subprogram(Subprogram {
            kind: SubprogramKind::Procedure,
            params,
            ret: None,
            generics: None,
            scope: None,
            body: Body::Pending,
            mangled: Symbol::intern("pr"),
            ret_mark: None,
            impure_reads: false,
        }),
        scope: fx.scope,
        span: Span::default(),
        implicit: false,
        since: LangRev::Vhdl1993,
    });
    let (ae, be) = (fx.obj(a), fx.obj(b));
    let span = fx.span();
    let call = Call {
        callee: pr,
        args: Box::new([
            Arg {
                formal: 0,
                actual: Actual::Expr(ae),
                inertial: false,
            },
            Arg {
                formal: 1,
                actual: Actual::Expr(be),
                inertial: false,
            },
        ]),
        method_of: None,
        span,
    };
    let c = fx.stmt(StmtKind::ConcProcCall {
        postponed: false,
        call,
        info: ProcessInfo::default(),
    });
    let info = fx.db.conc_info(c, None);
    assert_eq!(
        info.drivers.iter().map(|l| l.root).collect::<Vec<_>>(),
        [ObjRoot::Decl(b)]
    );
    assert_eq!(
        info.reads_all
            .unwrap()
            .iter()
            .map(|l| l.root)
            .collect::<Vec<_>>(),
        [ObjRoot::Decl(a)]
    );
}

#[test]
fn target_class_rules() {
    let mut fx = Fx::new();
    let bit = fx.bit;
    type Case = (
        ObjClass,
        ObjRole,
        Option<Mode>,
        TargetKind,
        Option<&'static str>,
    );
    let cases: Vec<Case> = vec![
        (
            ObjClass::Signal,
            ObjRole::Plain,
            None,
            TargetKind::Signal,
            None,
        ),
        (
            ObjClass::Variable,
            ObjRole::Plain,
            None,
            TargetKind::Variable,
            None,
        ),
        (
            ObjClass::Constant,
            ObjRole::Plain,
            None,
            TargetKind::Variable,
            Some("cannot assign to constant `x`"),
        ),
        (
            ObjClass::Constant,
            ObjRole::Plain,
            None,
            TargetKind::Signal,
            Some("cannot assign to constant `x`"),
        ),
        (
            ObjClass::Constant,
            ObjRole::LoopParam,
            None,
            TargetKind::Variable,
            Some("cannot assign to loop parameter `x`"),
        ),
        (
            ObjClass::Constant,
            ObjRole::Param,
            Some(Mode::In),
            TargetKind::Variable,
            Some("cannot assign to parameter `x` of mode `in`"),
        ),
        (
            ObjClass::Variable,
            ObjRole::Param,
            Some(Mode::In),
            TargetKind::Variable,
            Some("cannot assign to parameter `x` of mode `in`"),
        ),
        (
            ObjClass::Variable,
            ObjRole::Param,
            Some(Mode::Out),
            TargetKind::Variable,
            None,
        ),
        (
            ObjClass::Signal,
            ObjRole::Port,
            Some(Mode::In),
            TargetKind::Signal,
            Some("cannot assign to port `x` of mode `in`"),
        ),
        (
            ObjClass::Signal,
            ObjRole::Port,
            Some(Mode::Linkage),
            TargetKind::Signal,
            Some("cannot assign to port `x` of mode `linkage`"),
        ),
        (
            ObjClass::Signal,
            ObjRole::Port,
            Some(Mode::Out),
            TargetKind::Signal,
            None,
        ),
        (
            ObjClass::Signal,
            ObjRole::Port,
            Some(Mode::In),
            TargetKind::Force,
            None,
        ),
        (
            ObjClass::Variable,
            ObjRole::Plain,
            None,
            TargetKind::Signal,
            Some("`x` is a variable; use `:=` to assign it"),
        ),
        (
            ObjClass::Signal,
            ObjRole::Plain,
            None,
            TargetKind::Variable,
            Some("`x` is a signal; use `<=` to assign it"),
        ),
        (
            ObjClass::Variable,
            ObjRole::Plain,
            None,
            TargetKind::Force,
            Some("`force` target `x` must be a signal"),
        ),
        (
            ObjClass::Signal,
            ObjRole::Guard,
            None,
            TargetKind::Signal,
            Some("cannot assign to the implicit signal `x`"),
        ),
        (
            ObjClass::File,
            ObjRole::Plain,
            None,
            TargetKind::Variable,
            Some("cannot assign to file `x`"),
        ),
    ];
    for (class, role, mode, kind, want) in cases {
        let d = fx.object("x", class, role.clone(), mode, bit);
        let e = fx.obj(d);
        let before = fx.db.diagnostics().len();
        let r = fx.db.check_target(e, kind);
        let got = fx.db.diagnostics()[before..]
            .first()
            .map(|d| (d.code, d.message.clone()));
        match want {
            None => {
                assert!(r.is_ok(), "{class:?} {role:?} {mode:?}: {got:?}");
                assert_eq!(r.unwrap().len(), usize::from(kind.wants_signal()));
            }
            Some(m) => assert_eq!(
                got,
                Some((diag::CLASS_MISUSE, m.to_string())),
                "{class:?} {role:?} {mode:?}"
            ),
        }
    }
}

#[test]
fn aggregate_target_has_one_prefix_per_element() {
    let mut fx = Fx::new();
    let (bit, bv) = (fx.bit, fx.bv);
    let (a, b) = (fx.sig("a", bit), fx.sig("b", bit));
    let (ae, be) = (fx.obj(a), fx.obj(b));
    let agg = fx.expr(
        ExprKind::Aggregate {
            assocs: Box::new([
                AggAssoc {
                    choices: Box::new([]),
                    value: ae,
                    slice_piece: false,
                },
                AggAssoc {
                    choices: Box::new([]),
                    value: be,
                    slice_piece: false,
                },
            ]),
            bounds: AggBounds::Context,
        },
        bv,
        Staticness::None,
    );
    let lsps = fx.db.check_target(agg, TargetKind::Signal).unwrap();
    assert_eq!(
        lsps.iter().map(|l| l.root).collect::<Vec<_>>(),
        [ObjRoot::Decl(a), ObjRoot::Decl(b)]
    );
}

#[test]
fn sensitivity_rules() {
    let mut fx = Fx::new();
    let (bit, bv) = (fx.bit, fx.bv);
    let s = fx.sig("s", bv);
    let v = fx.object("v", ObjClass::Variable, ObjRole::Plain, None, bit);
    let l = fx.object(
        "l",
        ObjClass::Signal,
        ObjRole::Port,
        Some(Mode::Linkage),
        bit,
    );
    let so = fx.obj(s);
    assert!(fx.db.check_sensitivity(so, "a sensitivity list").is_ok());
    let ve = fx.obj(v);
    assert!(fx.db.check_sensitivity(ve, "a `wait on` list").is_err());
    let i = fx.opaque(Staticness::None);
    let so = fx.obj(s);
    let si = fx.index(so, i);
    assert!(fx.db.check_sensitivity(si, "a sensitivity list").is_err());
    let le = fx.obj(l);
    assert!(fx.db.check_sensitivity(le, "a sensitivity list").is_err());
    let got: Vec<_> = fx
        .db
        .diagnostics()
        .iter()
        .map(|d| (d.code, d.message.as_str()))
        .collect();
    assert_eq!(
        got,
        [
            (
                diag::CLASS_MISUSE,
                "`v` in a `wait on` list must be a signal"
            ),
            (
                diag::SENSITIVITY_NOT_STATIC,
                "the name in a sensitivity list is not a static name"
            ),
            (diag::CLASS_MISUSE, "cannot read port `l` of mode `linkage`"),
        ]
    );
}

#[test]
fn purity_rules() {
    for relaxed in [false, true] {
        let mut fx = Fx::new();
        fx.db.opts.relaxed = relaxed;
        let int = fx.int;
        let s = fx.sig("s", int);
        let c = fx.object("c", ObjClass::Constant, ObjRole::Plain, None, int);
        let (f, fscope) = add_subprogram(&mut fx.db, "f", SubprogramKind::Function { pure: true });
        let (g, _) = add_subprogram(&mut fx.db, "g", SubprogramKind::Function { pure: false });
        let local = fx.object("lv", ObjClass::Variable, ObjRole::Plain, None, int);
        fx.db.decls[local].scope = fscope;
        // return s; return c; return lv; return g;
        let call = fx.expr(
            ExprKind::Call {
                callee: g,
                args: Box::new([]),
                method_of: None,
                implicit_cond: false,
            },
            int,
            Staticness::None,
        );
        let mut body = Vec::new();
        for e in [fx.obj(s), fx.obj(c), fx.obj(local), call] {
            body.push(fx.stmt(StmtKind::Return(Some(e))));
        }
        fx.db.check_purity(f, fscope, &body);
        let got: Vec<_> = fx
            .db
            .diagnostics()
            .iter()
            .map(|d| (d.code, d.message.as_str()))
            .collect();
        let (e1, e2) = if relaxed {
            (diag::W_PURE_IMPURE, diag::W_PURE_IMPURE)
        } else {
            (diag::PURE_SIDE_EFFECT, diag::PURE_SIDE_EFFECT)
        };
        assert_eq!(
            got,
            [
                // In source order: the call expression was built first.
                (e1, "pure function `f` calls impure function `g`"),
                (e2, "pure function `f` reads signal `s` declared outside it"),
            ]
        );
        assert_eq!(fx.db.error_count(), if relaxed { 0 } else { 2 });
    }
}

#[test]
fn driver_outside_process_rules() {
    let mut fx = Fx::new();
    let bit = fx.bit;
    let s = fx.sig("s", bit);
    let p = fx.object("x", ObjClass::Signal, ObjRole::Param, Some(Mode::Out), bit);
    let (f, scope) = add_subprogram(&mut fx.db, "pr", SubprogramKind::Procedure);
    let mk = |in_process| StmtCx {
        scope,
        labels: scope,
        owner: Owner::Subprogram {
            decl: f,
            kind: SubprogramKind::Procedure,
            in_process,
            wait_ban: None,
        },
        loops: Vec::new(),
    };
    let (se, pe) = (fx.obj(s), fx.obj(p));
    let (ls, lp) = (fx.db.lsp(se).unwrap(), fx.db.lsp(pe).unwrap());
    assert!(
        fx.db
            .check_driver_outside_process(&mk(false), std::slice::from_ref(&lp))
            .is_ok()
    );
    assert!(
        fx.db
            .check_driver_outside_process(&mk(true), std::slice::from_ref(&ls))
            .is_ok()
    );
    assert!(
        fx.db
            .check_driver_outside_process(&mk(false), &[ls])
            .is_err()
    );
    assert_eq!(
        fx.db.diagnostics()[0].message,
        "signal `s` is driven from a subprogram and is not a parameter"
    );
}

#[test]
fn resolution_is_inherited_and_element_wise() {
    let fx = Fx::new();
    assert!(!fx.db.is_resolved(fx.bit));
    assert!(fx.db.is_resolved(fx.rbit));
    assert!(!fx.db.is_resolved(fx.bv));
    assert!(!fx.db.is_resolved(fx.rec), "element `a` is unresolved");
}

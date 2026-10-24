//! The implicit operation set per type class, against lists derived from the
//! LRM (clauses in the module doc of `implicit.rs`) and against the
//! `standard.vhdl` listing of both revisions.

use rustc_hash::FxHashMap as HashMap;

use super::*;
use crate::Options;
use crate::decl::Decl;
use crate::generic::GenericClass;
use crate::ids::{DeclId, LibId};
use crate::loader::{LibSource, Library, Unit, UnitKind};
use crate::scope::ScopeKind;
use crate::types::{PhysUnit, RecordElem, SubtypeDef, TypeDef};
use yavs_diag::FileId;

fn sp() -> Span {
    Span::new(FileId(0), 0, 1)
}

fn id(s: &str) -> Ident {
    Ident {
        text: Symbol::intern(s),
        key: Symbol::intern(&s.to_ascii_lowercase()),
        span: sp(),
    }
}

struct B {
    db: Db,
}

impl B {
    fn new(rev: LangRev) -> B {
        B {
            db: Db::new(Options::new(rev, "/nonexistent")),
        }
    }

    fn lib(&mut self, name: &str) -> LibId {
        let root = self.db.root;
        let decl = self.db.decls.push(Decl {
            name: Designator::Ident(id(name)),
            kind: DeclKind::Error,
            scope: root,
            span: sp(),
            implicit: false,
            since: LangRev::Vhdl2008,
        });
        self.db.libs.push(Library {
            name: id(name),
            decl,
            source: LibSource::Files,
            units: HashMap::default(),
        })
    }

    /// A package `lib.name` and its declarative region.
    fn package(&mut self, lib: &str, name: &str) -> ScopeId {
        let lib = self.lib(lib);
        let root = self.db.root;
        let unit = self.db.units.next_id();
        let decl = self.db.decls.push(Decl {
            name: Designator::Ident(id(name)),
            kind: DeclKind::Unit(unit),
            scope: root,
            span: sp(),
            implicit: false,
            since: LangRev::Vhdl2008,
        });
        let scope = self
            .db
            .add_scope(ScopeKind::Package, Some(root), Some(decl));
        let file = self.db.sources.add("t.vhd", "");
        self.db.units.push(Unit {
            lib,
            key: UnitKey::Primary(id(name).key),
            kind: UnitKind::Package,
            name: id(name),
            decl,
            scope,
            context: root,
            deps: Vec::new(),
            refs: Vec::new(),
            file,
            span: sp(),
            rev: self.db.rev,
            hir: crate::hir::UnitHir::Pending,
        });
        scope
    }

    /// `type name is <kind>` in `scope`, with its implicit operations.
    fn ty(&mut self, scope: ScopeId, name: &str, kind: TypeKind) -> TypeId {
        let ty = self.raw_ty(scope, name, kind, true);
        self.db.declare_implicit_ops(ty, scope);
        ty
    }

    fn raw_ty(&mut self, scope: ScopeId, name: &str, kind: TypeKind, visible: bool) -> TypeId {
        let ty = self.db.types.next_id();
        let decl = self.db.decls.push(Decl {
            name: Designator::Ident(id(name)),
            kind: DeclKind::Type(ty),
            scope,
            span: sp(),
            implicit: false,
            since: LangRev::Vhdl2008,
        });
        let st = self.db.subtypes.push(SubtypeDef {
            base: ty,
            name: Some(id(name)),
            parent: None,
            constraint: Constraint::None,
            resolution: None,
            span: sp(),
        });
        self.db.types.push(TypeDef {
            decl,
            name: id(name),
            anonymous: false,
            kind,
            first_subtype: st,
            base_subtype: st,
        });
        if visible {
            self.db.declare(scope, decl);
        }
        ty
    }

    /// Enumeration literals: `'c'` is a character literal, anything else an identifier.
    fn enum_ty(&mut self, scope: ScopeId, name: &str, lits: &[&str]) -> TypeId {
        let ty = self.db.types.next_id();
        let lits: Box<[DeclId]> = lits
            .iter()
            .enumerate()
            .map(|(pos, l)| {
                let name = match l.strip_prefix('\'') {
                    Some(c) => Designator::Char(c.chars().next().expect("char")),
                    None => Designator::Ident(id(l)),
                };
                self.db.decls.push(Decl {
                    name,
                    kind: DeclKind::EnumLit {
                        ty,
                        pos: pos as u32,
                    },
                    scope,
                    span: sp(),
                    implicit: false,
                    since: LangRev::Vhdl2008,
                })
            })
            .collect();
        self.ty(scope, name, TypeKind::Enum { lits })
    }

    fn subtype(&mut self, scope: ScopeId, name: &str, base: TypeId) -> SubtypeId {
        let st = self.db.subtypes.push(SubtypeDef {
            base,
            name: Some(id(name)),
            parent: None,
            constraint: Constraint::None,
            resolution: None,
            span: sp(),
        });
        let d = self.db.decls.push(Decl {
            name: Designator::Ident(id(name)),
            kind: DeclKind::Subtype(st),
            scope,
            span: sp(),
            implicit: false,
            since: LangRev::Vhdl2008,
        });
        self.db.declare(scope, d);
        st
    }

    fn first(&self, t: TypeId) -> SubtypeId {
        self.db.types[t].first_subtype
    }

    fn array(&mut self, scope: ScopeId, name: &str, index: SubtypeId, elem: TypeId) -> TypeId {
        let elem = self.first(elem);
        self.ty(
            scope,
            name,
            TypeKind::Array {
                indexes: Box::new([index]),
                elem,
            },
        )
    }

    /// `std.standard` in the order of `lib/std/<rev>/standard.vhdl`.
    fn standard(&mut self) -> (ScopeId, SubtypeId) {
        let rev = self.db.rev;
        let s = self.package("std", "standard");
        self.db.well_known.standard = Some(s);
        let ui = self.raw_ty(s, "universal_integer", TypeKind::UniversalInteger, false);
        let ur = self.raw_ty(s, "universal_real", TypeKind::UniversalReal, false);
        self.db.well_known.universal_integer = Some(ui);
        self.db.well_known.universal_real = Some(ur);
        self.db.declare_implicit_ops(ui, s);
        self.db.declare_implicit_ops(ur, s);
        if rev >= LangRev::Vhdl2019 {
            self.enum_ty(s, "RANGE_DIRECTION", &["ASCENDING", "DESCENDING"]);
        }
        let boolean = self.enum_ty(s, "BOOLEAN", &["FALSE", "TRUE"]);
        let bit = self.enum_ty(s, "BIT", &["'0", "'1"]);
        let character = self.enum_ty(s, "CHARACTER", &["NUL", "'a"]);
        self.enum_ty(s, "SEVERITY_LEVEL", &["NOTE", "FAILURE"]);
        let integer = self.ty(s, "INTEGER", TypeKind::Integer);
        let real = self.ty(s, "REAL", TypeKind::Floating);
        let time = self.ty(
            s,
            "TIME",
            TypeKind::Physical {
                units: Box::new([]),
            },
        );
        self.subtype(s, "DELAY_LENGTH", time);
        let natural = self.subtype(s, "NATURAL", integer);
        let positive = self.subtype(s, "POSITIVE", integer);
        self.array(s, "STRING", positive, character);
        self.array(s, "BOOLEAN_VECTOR", natural, boolean);
        self.array(s, "BIT_VECTOR", natural, bit);
        self.array(s, "INTEGER_VECTOR", natural, integer);
        self.array(s, "REAL_VECTOR", natural, real);
        self.array(s, "TIME_VECTOR", natural, time);
        self.enum_ty(s, "FILE_OPEN_KIND", &["READ_MODE", "WRITE_MODE"]);
        self.enum_ty(s, "FILE_OPEN_STATUS", &["OPEN_OK", "MODE_ERROR"]);
        if rev >= LangRev::Vhdl2019 {
            self.enum_ty(s, "FILE_OPEN_STATE", &["STATE_OPEN", "STATE_CLOSED"]);
            self.enum_ty(
                s,
                "FILE_ORIGIN_KIND",
                &["FILE_ORIGIN_BEGIN", "FILE_ORIGIN_END"],
            );
        }
        assert!(
            self.db.pending_implicit.is_empty(),
            "nothing waits after std.standard"
        );
        assert_eq!(self.db.error_count(), 0);
        (s, natural)
    }

    fn st_name(&self, st: SubtypeId) -> String {
        let s = &self.db.subtypes[st];
        match s.name {
            Some(n) => n.key.to_string(),
            None => self.db.types[s.base].name.key.to_string(),
        }
    }

    /// `name(t1, t2) return r`, lower case, parameter types only.
    fn sig(&self, d: DeclId) -> String {
        let decl = &self.db.decls[d];
        let DeclKind::Subprogram(sp) = &decl.kind else {
            panic!("not a subprogram");
        };
        let name = match decl.name {
            Designator::Operator { text, .. } => {
                format!("\"{}\"", text.as_str().to_ascii_lowercase())
            }
            n => n.key().to_string(),
        };
        let params: Vec<String> = sp.params.iter().map(|p| self.st_name(p.subtype)).collect();
        let mut s = format!("{name}({})", params.join(", "));
        if let Some(r) = sp.ret {
            s.push_str(&format!(" return {}", self.st_name(r)));
        }
        s
    }

    /// The implicit subprograms declared for `ty`, sorted.
    fn ops_of(&self, ty: TypeId) -> Vec<String> {
        let mut v: Vec<String> = self
            .db
            .decls
            .iter()
            .filter_map(|(d, decl)| match &decl.kind {
                DeclKind::Subprogram(Subprogram {
                    body: Body::Implicit(op),
                    ..
                }) if op.ty == ty => Some(self.sig(d)),
                _ => None,
            })
            .collect();
        v.sort();
        v
    }

    fn all_ops(&self, scope: ScopeId) -> Vec<String> {
        let mut v: Vec<String> = self.db.scopes[scope]
            .order
            .iter()
            .filter(|&&d| matches!(self.db.decls[d].kind, DeclKind::Subprogram(_)))
            .map(|&d| self.sig(d))
            .collect();
        v.sort();
        v
    }
}

/// The commented `-- function ...` declarations of `standard.vhdl`, in `B::sig` form.
fn listing(rev: LangRev) -> Vec<String> {
    let path = crate::default_stdlib().join(format!("std/{}/standard.vhdl", rev.as_str()));
    let bytes = crate::stdlib::read(&path).expect("standard.vhdl");
    let text: String = bytes.iter().map(|&b| b as char).collect();
    let mut v = Vec::new();
    for line in text.lines() {
        let Some(f) = line.trim().strip_prefix("--").map(str::trim) else {
            continue;
        };
        let Some(f) = f.strip_prefix("function ") else {
            continue;
        };
        let f = f.to_ascii_lowercase();
        let open = f.find('(').expect("(");
        let mut name = f[..open].trim().to_string();
        if name.starts_with('"') && !name[1..].contains('"') {
            name.push('"'); // `"- (anonymous: TIME)` in the 2019 listing
        }
        let close = f.rfind(')').expect(")");
        let mut types = Vec::new();
        for group in f[open + 1..close].split(';') {
            let (names, ty) = group.split_once(':').expect(":");
            let ty = ty.split_whitespace().last().expect("type").to_string();
            types.extend(names.split(',').map(|_| ty.clone()));
        }
        let ret = f[close + 1..]
            .trim()
            .strip_prefix("return")
            .expect("return")
            .trim()
            .trim_end_matches(';')
            .to_string();
        v.push(format!("{name}({}) return {ret}", types.join(", ")));
    }
    v.sort();
    v
}

fn diff(got: &[String], want: &[String]) -> (Vec<String>, Vec<String>) {
    let mut extra = got.to_vec();
    let mut missing = Vec::new();
    for w in want {
        match extra.iter().position(|g| g == w) {
            Some(i) => {
                extra.remove(i);
            }
            None => missing.push(w.clone()),
        }
    }
    (extra, missing)
}

fn strings(v: &[&str]) -> Vec<String> {
    let mut v: Vec<String> = v.iter().map(|s| s.to_string()).collect();
    v.sort();
    v
}

#[test]
fn standard_matches_the_listing() {
    for rev in [LangRev::Vhdl2008, LangRev::Vhdl2019] {
        let mut b = B::new(rev);
        let (s, _) = b.standard();
        let got = b.all_ops(s);
        let (extra, missing) = diff(&got, &listing(rev));
        // Where the informative listing and the normative text disagree:
        // 9.2.3: `?=` and `?/=` are predefined for arrays of BIT and STD_ULOGIC only (sim1, sim2 agree).
        let mut want_missing = vec![
            "\"?/=\"(boolean_vector, boolean_vector) return boolean",
            "\"?=\"(boolean_vector, boolean_vector) return boolean",
        ];
        let want_extra = if rev == LangRev::Vhdl2008 {
            // 5.3.2.4: TO_STRING on arrays only for elements with character literals only
            // (CHARACTER has NUL etc.; sim1 and sim2 reject `to_string(string)` in 2008).
            want_missing.extend([
                "to_string(boolean_vector) return string",
                "to_string(integer_vector) return string",
                "to_string(real_vector) return string",
                "to_string(string) return string",
                "to_string(time_vector) return string",
            ]);
            vec![]
        } else {
            let mut v = vec![
                // 2019 5.7: TO_STRING on every scalar type, the listing omits the new types.
                "to_string(range_direction) return string",
                "to_string(file_open_state) return string",
                "to_string(file_origin_kind) return string",
            ];
            // LCS-2016-059a: relational operators on arrays of any scalar element.
            for t in ["real_vector", "time_vector"] {
                for op in ["\"<\"", "\"<=\"", "\">\"", "\">=\""] {
                    v.push(Box::leak(
                        format!("{op}({t}, {t}) return boolean").into_boxed_str(),
                    ));
                }
                for f in ["minimum", "maximum"] {
                    v.push(Box::leak(
                        format!("{f}({t}, {t}) return {t}").into_boxed_str(),
                    ));
                }
            }
            v
        };
        assert_eq!(
            extra,
            strings(&want_extra),
            "{rev:?}: created but not listed"
        );
        assert_eq!(
            missing,
            strings(&want_missing),
            "{rev:?}: listed but not created"
        );

        // Aliases of BIT_VECTOR's string conversions (5.3.2.4).
        for (alias, target) in [
            ("to_bstring", "to_string"),
            ("to_binary_string", "to_string"),
            ("to_octal_string", "to_ostring"),
            ("to_hex_string", "to_hstring"),
        ] {
            let d = b.db.scopes[s].names[&Symbol::intern(alias)][0];
            let t = b.db.entity(d);
            assert_eq!(b.sig(t), format!("{target}(bit_vector) return string"));
        }
    }
}

#[test]
fn user_types_per_class() {
    for rev in [LangRev::Vhdl2008, LangRev::Vhdl2019] {
        let v19 = rev == LangRev::Vhdl2019;
        let mut b = B::new(rev);
        let (std, natural) = b.standard();
        let get = |b: &B, n: &str| {
            let d = b.db.scopes[std].names[&Symbol::intern(n)][0];
            match b.db.decls[d].kind {
                DeclKind::Type(t) => t,
                _ => panic!("{n}"),
            }
        };
        let bit = get(&b, "bit");
        let p = b.package("work", "p");
        let check = |b: &B, t: TypeId, want: Vec<String>| {
            let mut want = want;
            want.sort();
            assert_eq!(b.ops_of(t), want, "{rev:?} {}", b.db.types[t].name.text);
        };
        let ops = |t: &str, list: &[&str]| -> Vec<String> {
            list.iter().map(|s| s.replace('T', t)).collect()
        };
        let eq = ["\"=\"(T, T) return boolean", "\"/=\"(T, T) return boolean"];
        let scalar = [
            &eq[..],
            &[
                "\"<\"(T, T) return boolean",
                "\"<=\"(T, T) return boolean",
                "\">\"(T, T) return boolean",
                "\">=\"(T, T) return boolean",
                "minimum(T, T) return T",
                "maximum(T, T) return T",
                "to_string(T) return string",
            ],
        ]
        .concat();
        let num = [
            "\"+\"(T) return T",
            "\"-\"(T) return T",
            "\"abs\"(T) return T",
            "\"+\"(T, T) return T",
            "\"-\"(T, T) return T",
        ];

        // 5.2.6, 9.2: enumeration types, identifiers and characters alike.
        let e = b.enum_ty(p, "e", &["a", "b"]);
        check(&b, e, ops("e", &scalar));
        let c = b.enum_ty(p, "c", &["'x", "'y"]);
        check(&b, c, ops("c", &scalar));

        // 9.2.5-9.2.8: integer and floating types.
        let i = b.ty(p, "i", TypeKind::Integer);
        let int_ops = [
            &scalar[..],
            &num[..],
            &[
                "\"*\"(T, T) return T",
                "\"/\"(T, T) return T",
                "\"mod\"(T, T) return T",
                "\"rem\"(T, T) return T",
                "\"**\"(T, integer) return T",
            ],
        ]
        .concat();
        check(&b, i, ops("i", &int_ops));
        let f = b.ty(p, "f", TypeKind::Floating);
        let float_ops: Vec<&str> = int_ops
            .iter()
            .copied()
            .filter(|s| !s.contains("mod") && !s.contains("rem"))
            .collect();
        check(&b, f, ops("f", &float_ops));

        // 5.2.4, 9.2.7: physical types.
        let ph = b.ty(
            p,
            "ph",
            TypeKind::Physical {
                units: Box::new([PhysUnit {
                    decl: b.db.poison.decl,
                    scale: 1,
                }]),
            },
        );
        let phys_ops = [
            &scalar[..],
            &num[..],
            &[
                "\"*\"(T, integer) return T",
                "\"*\"(T, real) return T",
                "\"*\"(integer, T) return T",
                "\"*\"(real, T) return T",
                "\"/\"(T, integer) return T",
                "\"/\"(T, real) return T",
                "\"/\"(T, T) return universal_integer",
                "\"mod\"(T, T) return T",
                "\"rem\"(T, T) return T",
            ],
        ]
        .concat();
        check(&b, ph, ops("ph", &phys_ops));

        // 5.3.2.4, 9.2.3: one-dimensional arrays.
        let cat = [
            "\"&\"(T, T) return T",
            "\"&\"(T, E) return T",
            "\"&\"(E, T) return T",
            "\"&\"(E, E) return T",
            "minimum(T) return E",
            "maximum(T) return E",
        ];
        let ordered = [
            "\"<\"(T, T) return boolean",
            "\"<=\"(T, T) return boolean",
            "\">\"(T, T) return boolean",
            "\">=\"(T, T) return boolean",
            "minimum(T, T) return T",
            "maximum(T, T) return T",
        ];
        let arr =
            |t: &str, e: &str, ordered_ops: bool, to_string: bool, extra: &[&str]| -> Vec<String> {
                let mut v: Vec<&str> = [&eq[..], &cat[..], extra].concat();
                if ordered_ops {
                    v.extend(ordered);
                }
                if to_string {
                    v.push("to_string(T) return string");
                }
                v.iter()
                    .map(|s| s.replace('E', e).replace('T', t))
                    .collect()
            };
        let cv = b.array(p, "cv", natural, c);
        check(&b, cv, arr("cv", "c", true, true, &[]));
        let iv = b.array(p, "iv", natural, i);
        check(&b, iv, arr("iv", "i", true, v19, &[]));
        let fv = b.array(p, "fv", natural, f);
        check(&b, fv, arr("fv", "f", v19, v19, &[]));
        // 9.2.2, 9.2.4, 9.2.3: arrays of BIT get logical, shift and matching operators.
        let bv = b.array(p, "bv", natural, bit);
        let mut logic = Vec::new();
        for op in ["and", "or", "nand", "nor", "xor", "xnor"] {
            for form in [
                "(T, T) return T",
                "(T, E) return T",
                "(E, T) return T",
                "(T) return E",
            ] {
                logic.push(format!("\"{op}\"{form}"));
            }
        }
        logic.push("\"not\"(T) return T".into());
        for op in ["sll", "srl", "sla", "sra", "rol", "ror"] {
            logic.push(format!("\"{op}\"(T, integer) return T"));
        }
        logic.push("\"?=\"(T, T) return E".into());
        logic.push("\"?/=\"(T, T) return E".into());
        let logic: Vec<&str> = logic.iter().map(String::as_str).collect();
        check(&b, bv, arr("bv", "bit", true, true, &logic));
        // Multi-dimensional arrays: only equality (5.3.2.4).
        let m = b.ty(
            p,
            "m",
            TypeKind::Array {
                indexes: Box::new([natural, natural]),
                elem: b.first(bit),
            },
        );
        let mut want = ops("m", &eq);
        if v19 {
            want.push("to_string(m) return string".into());
        }
        check(&b, m, want);

        // 5.3.3: records; 2019 5.7 TO_STRING when every element has it.
        let r = b.ty(
            p,
            "r",
            TypeKind::Record {
                elems: Box::new([RecordElem {
                    name: id("x"),
                    subtype: b.first(i),
                }]),
            },
        );
        let mut want = ops("r", &eq);
        if v19 {
            want.push("to_string(r) return string".into());
        }
        check(&b, r, want);

        // 5.4.3: access types.
        let a = b.ty(
            p,
            "a",
            TypeKind::Access {
                designated: b.first(i),
            },
        );
        let mut want = ops("a", &eq);
        want.push("deallocate(a)".into());
        check(&b, a, want);

        // 5.5.2: file types; READ has LENGTH for an unconstrained array element.
        let string = get(&b, "string");
        for (name, elem, read) in [
            ("fs", b.first(string), "read(fs, string, natural)"),
            ("fi", b.first(i), "read(fi, i)"),
        ] {
            let ft = b.ty(p, name, TypeKind::File { elem });
            let e = b.st_name(elem);
            let mut want: Vec<String> = [
                "file_open(T, string, file_open_kind)",
                "file_open(file_open_status, T, string, file_open_kind)",
                "file_close(T)",
                "write(T, E)",
                "flush(T)",
                "endfile(T) return boolean",
            ]
            .iter()
            .map(|s| s.replace('E', &e).replace('T', name))
            .collect();
            want.push(read.into());
            if v19 {
                want.extend(
                    [
                        "file_open(T, string, file_open_kind) return file_open_status",
                        "file_rewind(T)",
                        "file_seek(T, integer, file_origin_kind)",
                        "file_truncate(T, integer, file_origin_kind)",
                        "file_state(T) return file_open_state",
                        "file_mode(T) return file_open_kind",
                        "file_position(T, file_origin_kind) return integer",
                        "file_size(T) return integer",
                        "file_canseek(T) return boolean",
                    ]
                    .iter()
                    .map(|s| s.replace('T', name)),
                );
            }
            check(&b, ft, want);
        }

        // Protected types have no implicit operations (5.6.1).
        let pt_scope = b.db.add_scope(ScopeKind::Protected, Some(p), None);
        let pt = b.ty(
            p,
            "pt",
            TypeKind::Protected {
                decl_scope: pt_scope,
                body: None,
                generics: None,
            },
        );
        check(&b, pt, vec![]);

        // 6.5.3: a 2008 interface type and a 2019 private type get `=` and `/=` (Q9).
        let g = b.ty(
            p,
            "g",
            TypeKind::Generic {
                class: GenericClass::Private,
            },
        );
        check(&b, g, ops("g", &eq));
        let gi = b.ty(
            p,
            "gi",
            TypeKind::Generic {
                class: GenericClass::Integer,
            },
        );
        check(&b, gi, ops("gi", &int_ops));

        // 9.2.3: matching operators on STD_ULOGIC and its arrays; its logical
        // operators and `??` are explicit in std_logic_1164.
        let ieee = b.package("ieee", "std_logic_1164");
        let u = b.enum_ty(ieee, "STD_ULOGIC", &["'U", "'1"]);
        let mut want = ops("std_ulogic", &scalar);
        for op in ["?=", "?/=", "?<", "?<=", "?>", "?>="] {
            want.push(format!(
                "\"{op}\"(std_ulogic, std_ulogic) return std_ulogic"
            ));
        }
        check(&b, u, want);
        let uv = b.array(ieee, "STD_ULOGIC_VECTOR", natural, u);
        let m: &[&str] = &["\"?=\"(T, T) return E", "\"?/=\"(T, T) return E"];
        check(
            &b,
            uv,
            arr("std_ulogic_vector", "std_ulogic", true, true, m),
        );
        assert_eq!(b.db.error_count(), 0);
    }
}

#[test]
fn natives_names_and_revisions() {
    let mut b = B::new(LangRev::Vhdl2019);
    let (s, _) = b.standard();
    let p = b.package("work", "p");
    let i = b.ty(p, "i", TypeKind::Integer);
    let ft = b.ty(p, "fi", TypeKind::File { elem: b.first(i) });
    let mut natives = Vec::new();
    let mut mangled = Vec::new();
    for (d, decl) in b.db.decls.iter() {
        if let DeclKind::Subprogram(sp) = &decl.kind {
            let Body::Implicit(op) = &sp.body else {
                continue;
            };
            if op.ty == i || op.ty == ft {
                natives.push(crate::mangle::implicit_native(&b.db, op));
                mangled.push(sp.mangled.to_string());
                if op.family == Family::FileState {
                    assert_eq!(decl.since, LangRev::Vhdl2019, "{}", b.sig(d));
                }
                if op.family == Family::Eq {
                    assert_eq!(decl.since, LangRev::Vhdl2008);
                    assert!(decl.implicit);
                }
            }
        }
    }
    for want in [
        "yavs.implicit.eq.work.p.i",
        "yavs.implicit.exp.work.p.i",
        "yavs.implicit.file_open.work.p.fi",
        "yavs.implicit.file_open.work.p.fi#status",
        "yavs.implicit.file_open.work.p.fi#function",
    ] {
        assert!(natives.iter().any(|n| n == want), "{want} in {natives:#?}");
    }
    let unique: std::collections::HashSet<_> = natives.iter().collect();
    assert_eq!(
        unique.len(),
        natives.len(),
        "native symbols are distinct per type"
    );
    assert!(
        mangled
            .iter()
            .any(|m| m == "work.p.\"=\"(work.p.i,work.p.i)std.standard.boolean"),
        "{mangled:#?}"
    );
    assert!(
        mangled
            .iter()
            .any(|m| m == "work.p.\"**\"(work.p.i,std.standard.integer)work.p.i")
    );

    // Universal operators exist before INTEGER (its range uses unary `-`) and
    // every native symbol in std.standard is distinct.
    let neg = b.db.scopes[s].names[&Symbol::intern("\"-\"")]
        .iter()
        .any(|&d| b.sig(d) == "\"-\"(universal_integer) return universal_integer");
    assert!(neg);
    let mut all = std::collections::HashSet::new();
    for &d in &b.db.scopes[s].order {
        if let DeclKind::Subprogram(Subprogram {
            body: Body::Implicit(op),
            ..
        }) = &b.db.decls[d].kind
        {
            assert!(
                all.insert(crate::mangle::implicit_native(&b.db, op)),
                "{}",
                b.sig(d)
            );
        }
    }

    // FILE_OPEN's Open_Kind defaults to READ_MODE; parameter names are the LRM's.
    let fo = b.db.scopes[p].names[&Symbol::intern("file_open")][0];
    let DeclKind::Subprogram(sp) = &b.db.decls[fo].kind else {
        unreachable!()
    };
    let names: Vec<&str> = sp.params.iter().map(|q| q.name.text.as_str()).collect();
    assert_eq!(names, ["F", "External_Name", "Open_Kind"]);
    assert_eq!(sp.params[0].class, ObjClass::File);
    let def = sp.params[2].default.expect("default");
    assert_eq!(b.db.value(def), Some(&ConstValue::Enum(0)));
    // Operator operands are anonymous: no source name reaches them.
    let eq = b.db.scopes[p].names[&Symbol::intern("\"=\"")][0];
    let DeclKind::Subprogram(sp) = &b.db.decls[eq].kind else {
        unreachable!()
    };
    assert_eq!(sp.params[0].name.key.as_str(), "<anonymous>");
}

#[test]
fn explicit_declaration_hides_the_implicit_one() {
    let mut b = B::new(LangRev::Vhdl2008);
    b.standard();
    let p = b.package("work", "p");
    let i = b.ty(p, "i", TypeKind::Integer);
    let implicit = b.db.scopes[p].names[&Symbol::intern("\"=\"")][0];
    let DeclKind::Subprogram(s) = &b.db.decls[implicit].kind else {
        unreachable!()
    };
    let (params, ret) = (
        s.params.iter().map(|q| q.subtype).collect::<Vec<_>>(),
        s.ret,
    );
    let explicit = b.db.decls.push(Decl {
        name: Designator::operator("="),
        kind: DeclKind::Subprogram(Subprogram {
            kind: SubprogramKind::Function { pure: true },
            params: params
                .iter()
                .map(|&subtype| Param {
                    name: id("l"),
                    class: ObjClass::Constant,
                    mode: Mode::In,
                    subtype,
                    default: None,
                    decl: None,
                    mark: None,
                })
                .collect(),
            ret,
            generics: None,
            scope: None,
            body: Body::Pending,
            mangled: Symbol::intern("x"),
            ret_mark: None,
            impure_reads: false,
        }),
        scope: p,
        span: sp(),
        implicit: false,
        since: LangRev::Vhdl2008,
    });
    b.db.declare(p, explicit);
    assert_eq!(
        b.db.scopes[p].names[&Symbol::intern("\"=\"")],
        vec![explicit]
    );
    let _ = i;
}

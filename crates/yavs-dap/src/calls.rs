//! Function calls, type conversions and operators a type declares (`numeric_std."+"`) in the
//! debugger's expressions (#364). The subprogram is found by visibility from the region of the
//! stopped process and chosen by the types of the arguments; it runs lowered on its own
//! (`yavs_elab::Isolated`) on copies of what it reads (the signals and constants of its regions,
//! `now`), with no kernel or shared state, so a call cannot change the simulation. Pure and
//! impure functions are called if their code, and that of what they call, writes nothing
//! (`Isolated::effect`); one that may write is refused with the reason.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use yavs_elab::{Isolated, Val};
use yavs_sem::decl::{Body, DeclKind, ObjClass, SubprogramKind, operator_key};
use yavs_sem::scope::UseItem;
use yavs_sem::{Db, DeclId, ScopeId, TypeId};
use yavs_std::value::{Lit, LitType, literal};
use yavs_syntax::Symbol;

use crate::cond::{Arg, Called};
use crate::design::{fits, from_val, literal_type, to_val};
use crate::{At, Shared};

/// The functions lowered for calls so far, by declaration.
#[derive(Default)]
pub(crate) struct Lowered(Mutex<HashMap<DeclId, Arc<Result<Isolated, String>>>>);

impl Lowered {
    fn get(&self, db: &Db, f: DeclId) -> Arc<Result<Isolated, String>> {
        let mut m = self.0.lock().unwrap_or_else(|p| p.into_inner());
        m.entry(f)
            .or_insert_with(|| Arc::new(Isolated::new(db, f)))
            .clone()
    }
}

/// Where names are seen from: the sem region of the process (or of the region it is in), and the
/// elaborated region whose values its objects have.
#[derive(Clone, Copy, Default)]
pub(crate) struct Site {
    pub scope: Option<ScopeId>,
    pub inst: Option<yavs_elab::ScopeId>,
}

/// The declarations named `key` visible from `scope`, inner regions first. Read only:
/// `Db::lookup` takes the database mutably (its cache, units it loads).
fn visible(db: &Db, scope: ScopeId, key: Symbol) -> Vec<DeclId> {
    let mut out: Vec<DeclId> = Vec::new();
    let mut s = Some(scope);
    while let Some(sc) = s {
        let r = &db.scopes[sc];
        out.extend(r.names.get(&key).into_iter().flatten());
        for u in &r.uses {
            match u {
                UseItem::All(p) => out.extend(db.scopes[*p].names.get(&key).into_iter().flatten()),
                UseItem::One(ds) => {
                    out.extend(ds.iter().filter(|d| db.decls[**d].name.key() == key))
                }
                UseItem::Library(_) => {}
            }
        }
        s = r.parent;
    }
    let mut seen = std::collections::HashSet::new();
    out.retain(|d| seen.insert(*d));
    out
}

/// The type a declaration names, for a conversion.
fn type_of(db: &Db, d: DeclId) -> Option<TypeId> {
    match &db.decls[d].kind {
        DeclKind::Type(t) => Some(*t),
        DeclKind::Subtype(s) => Some(db.base(*s)),
        _ => None,
    }
}

fn type_name(db: &Db, t: TypeId) -> String {
    db.types[t].name.text.to_string()
}

/// The number of elements of an array literal: a string or a bit string.
fn literal_len(s: &str) -> Option<u64> {
    let s = s.trim();
    let q = s.find('"')?;
    let inner = s[q..].strip_prefix('"')?.strip_suffix('"')?;
    if q == 0 {
        return Some(inner.replace("\"\"", "\"").chars().count() as u64);
    }
    let pre = s[..q].to_ascii_lowercase();
    let size = pre.trim_end_matches(|c: char| c.is_ascii_alphabetic());
    if !size.is_empty() {
        return size.parse().ok();
    }
    let digits = inner.chars().filter(|&c| c != '_').count() as u64;
    match pre.trim_start_matches(['s', 'u']) {
        "b" => Some(digits),
        "o" => Some(digits * 3),
        "x" => Some(digits * 4),
        _ => None,
    }
}

/// A literal read in sem base type `ty`.
fn read_literal(db: &Db, ty: TypeId, s: &str) -> Option<(LitType, Lit)> {
    let len = match &db.types[ty].kind {
        yavs_sem::types::TypeKind::Array { .. } => literal_len(s)?,
        _ => 0,
    };
    let t = literal_type(db, ty, len)?;
    let v = literal(s.trim().as_bytes(), &t).ok()?;
    Some((t, v))
}

/// How well `args` fit the parameters of function `f`: 2 per argument of the same type name, 1
/// per argument that fits, 0 per value not known yet; `None` when one does not fit.
fn score(db: &Db, f: DeclId, args: &[Arg<'_>]) -> Option<u32> {
    let sp = db.decls[f].kind.subprogram()?;
    let mut n = 0;
    for (p, a) in sp.params.iter().zip(args) {
        if matches!(p.class, ObjClass::Signal | ObjClass::File) {
            return None;
        }
        let pt = db.base(p.subtype);
        n += match a {
            Arg::Value(t, _, Some(name)) => {
                (type_name(db, pt).eq_ignore_ascii_case(name) && fits(db, pt, t)).then_some(2)?
            }
            Arg::Value(t, _, None) => fits(db, pt, t).then_some(1)?,
            Arg::Literal(s) => read_literal(db, pt, s).map(|_| 1)?,
            Arg::Unknown => 0,
        };
    }
    Some(n)
}

/// A function as a user writes its profile: `to_integer(unresolved_unsigned) return natural`.
fn profile(db: &Db, f: DeclId) -> String {
    let d = &db.decls[f];
    let Some(sp) = d.kind.subprogram() else {
        return d.name.to_string();
    };
    let ps: Vec<String> = sp
        .params
        .iter()
        .map(|p| type_name(db, db.base(p.subtype)))
        .collect();
    let ret = sp.ret.map_or(String::new(), |r| {
        format!(" return {}", type_name(db, db.base(r)))
    });
    format!("{}({}){ret}", d.name, ps.join(", "))
}

/// The value of object `d` (a constant or generic of the regions around a function) seen from
/// elaborated region `inst`.
fn region_value(sh: &Shared, inst: Option<yavs_elab::ScopeId>, d: DeclId) -> Option<Val> {
    let e = &sh.elab;
    let mut s = inst;
    while let Some(sc) = s {
        if let Some(v) = e.model_vals.get(sc.index()).and_then(|m| m.get(&d)) {
            return Some(v.clone());
        }
        if let Some(v) = e.region_vals.get(&(Some(sc), d)) {
            return Some(v.clone());
        }
        s = e.hier.scopes.get(sc.index()).and_then(|x| x.parent);
    }
    e.pkg_vals
        .get(&d)
        .or_else(|| e.region_vals.get(&(None, d)))
        .cloned()
}

/// `T(x)`: the value of `a` as one of type `t`.
fn convert(db: &Db, name: &str, t: TypeId, a: &Arg<'_>) -> Result<Called, String> {
    use yavs_sem::types::TypeKind as K;
    let tn = Some(type_name(db, t));
    let no = || format!("`{name}(...)`: this value does not convert to `{name}`");
    match a {
        Arg::Unknown => Ok(None),
        Arg::Literal(s) => match read_literal(db, t, s) {
            Some((lt, v)) => Ok(Some((lt, v, tn))),
            // A number of the other numeric kind.
            None => match (
                &db.types[t].kind,
                literal(
                    s.as_bytes(),
                    &LitType::Real(yavs_std::types::RealBounds::REAL),
                ),
            ) {
                (K::Integer, Ok(Lit::Real(x))) => round(x)
                    .map(|i| Some((LitType::Int(yavs_std::types::Bounds::I64), Lit::Int(i), tn))),
                _ => Err(no()),
            },
        },
        Arg::Value(lt, v, _) => match (&db.types[t].kind, v) {
            (K::Integer, Lit::Real(x)) => round(*x)
                .map(|i| Some((LitType::Int(yavs_std::types::Bounds::I64), Lit::Int(i), tn))),
            (K::Floating, Lit::Int(i)) => Ok(Some((
                LitType::Real(yavs_std::types::RealBounds::REAL),
                Lit::Real(*i as f64),
                tn,
            ))),
            _ if fits(db, t, lt) => Ok(Some(((*lt).clone(), (*v).clone(), tn))),
            _ => Err(no()),
        },
    }
}

/// A real rounded to the nearest integer, halves away from zero (LRM 9.3.6).
fn round(x: f64) -> Result<i64, String> {
    let r = x.round();
    if r.is_finite() && r >= i64::MIN as f64 && r < i64::MAX as f64 {
        Ok(r as i64)
    } else {
        Err("overflow in the condition".into())
    }
}

/// A call of `name` (an operator as `"+"`) on `args` seen from `site`, run with the signals and
/// time of stop `at`; without one it is only resolved and checked, as when a condition is set.
/// `None`: nothing of that name takes these arguments here (a name without arguments is no
/// function), and an operator keeps its predefined meaning.
pub(crate) fn call(
    sh: &Shared,
    site: &Site,
    name: &str,
    args: &[Arg<'_>],
    at: Option<&At<'_, '_>>,
) -> Option<Result<Called, String>> {
    let db = &*sh.db;
    let Some(scope) = site.scope else {
        // Statements of a subprogram: its caller decides what is visible (a word stays a word).
        return (at.is_none() && !args.is_empty()).then_some(Ok(None));
    };
    let (qual, simple) = match name.rsplit_once('.') {
        Some((q, s)) if !name.starts_with('"') => (Some(q), s),
        _ => (None, name),
    };
    let op = simple.strip_prefix('"').and_then(|s| s.strip_suffix('"'));
    let key = match op {
        Some(o) => operator_key(&o.to_ascii_lowercase()),
        None => Symbol::intern(&simple.to_ascii_lowercase()),
    };
    let mut decls = visible(db, scope, key);
    if let Some(q) = qual {
        let last = q.rsplit('.').next().unwrap_or(q);
        decls.retain(|d| {
            let owner = db.scopes[db.decls[*d].scope].owner;
            owner.is_some_and(|o| db.decls[o].name.to_string().eq_ignore_ascii_case(last))
        });
    }
    if op.is_none()
        && let [a] = args
        && let Some(t) = decls.iter().find_map(|d| type_of(db, *d))
    {
        return Some(convert(db, simple, t, a));
    }
    let shown = op.map_or_else(|| simple.to_string(), |o| format!("\"{o}\""));
    let funcs: Vec<DeclId> = decls
        .iter()
        .copied()
        .filter(|&d| {
            db.decls[d].kind.subprogram().is_some_and(|sp| {
                matches!(sp.kind, SubprogramKind::Function { .. })
                    && !matches!(sp.body, Body::Implicit(_))
                    && sp.params.len() == args.len()
            })
        })
        .collect();
    if funcs.is_empty() {
        // Nothing of the name: the caller's message for an unknown name, with its suggestions.
        // A name without arguments reads as a literal.
        if op.is_some() || decls.is_empty() || args.is_empty() {
            return None;
        }
        return Some(Err(format!(
            "`{simple}` here is no function of {} argument{}",
            args.len(),
            if args.len() == 1 { "" } else { "s" }
        )));
    }
    let scored: Vec<(DeclId, u32)> = funcs
        .iter()
        .filter_map(|&f| Some((f, score(db, f, args)?)))
        .collect();
    let Some(top) = scored.iter().map(|s| s.1).max() else {
        if op.is_some() {
            return None;
        }
        let signal = |f: &DeclId| {
            let sp = db.decls[*f].kind.subprogram();
            sp.is_some_and(|sp| sp.params.iter().any(|p| p.class == ObjClass::Signal))
        };
        if funcs.iter().all(signal) {
            return Some(Err(format!(
                "`{shown}` takes a signal parameter, which a condition cannot pass (it reads \
                 values only): write the test with attributes, as in clk'event and clk = '1'"
            )));
        }
        let cands: Vec<String> = funcs.iter().map(|&f| profile(db, f)).collect();
        let given: Vec<&str> = args
            .iter()
            .map(|a| match a {
                Arg::Value(_, _, Some(n)) => n,
                Arg::Value(LitType::Int(_), ..) => "an integer",
                Arg::Value(LitType::Real(_), ..) => "a real",
                Arg::Value(..) => "a value",
                Arg::Literal(_) => "a literal",
                Arg::Unknown => "a local",
            })
            .collect();
        return Some(Err(format!(
            "no `{shown}` here takes ({}); there are {}",
            given.join(", ").to_ascii_lowercase(),
            cands.join(", ")
        )));
    };
    let best: Vec<DeclId> = scored.iter().filter(|s| s.1 == top).map(|s| s.0).collect();
    let unknown = args.iter().any(|a| matches!(a, Arg::Unknown));
    if unknown {
        return Some(Ok(None));
    }
    // Operands whose types are not known (a field, a variable of a subprogram) that fit several
    // operators keep the predefined meaning.
    if best.len() > 1 && op.is_some() {
        return None;
    }
    if best.len() > 1 {
        let cands: Vec<String> = best.iter().map(|&f| profile(db, f)).collect();
        return Some(Err(format!(
            "`{shown}` is ambiguous here: {} (the types of the arguments do not tell them apart; \
             convert one, as in unsigned(x))",
            cands.join(", ")
        )));
    }
    let f = best[0];
    let Some(sp) = db.decls[f].kind.subprogram() else {
        return Some(Err(
            "this is a bug in yavs: a function that is no subprogram".into(),
        ));
    };
    if args.is_empty()
        && decls
            .iter()
            .any(|&d| db.decls[d].kind.subprogram().is_none())
    {
        // An enumeration literal of the same name: the literal.
        return None;
    }
    if matches!(sp.body, Body::Foreign(_)) {
        return Some(Err(format!(
            "`{shown}` is a foreign function: conditions do not call it"
        )));
    }
    if (sp.params.iter()).any(|p| !matches!(p.mode, yavs_sem::hir::Mode::In)) {
        return Some(Err(format!(
            "`{shown}` has a parameter of mode out or inout, which writes what it is given: \
             conditions do not call it"
        )));
    }
    if let Body::Native(sym) = &sp.body {
        // `now`, the time of the stop; the other subprograms of std without a VHDL body change
        // or read the state of the simulation or of the host.
        if !sym.as_str().starts_with("yavs.standard.now.") {
            return Some(Err(format!(
                "`{shown}` is built into the simulator: conditions do not call it"
            )));
        }
        let Some(at) = at else {
            return Some(Ok(None));
        };
        let t = at.now().ok_or("`now` has no value at this stop");
        return Some(
            t.map(|t| {
                let fs = i64::try_from(t.as_fs()).unwrap_or(i64::MAX);
                let ty = LitType::Phys(yavs_std::types::time_type(), yavs_std::types::Bounds::I64);
                Some((ty, Lit::Phys(fs), None))
            })
            .map_err(String::from),
        );
    }
    // ponytail: a function of a process or subprogram needs the frame of its parent (a static
    // link), which a call made apart from the simulation does not have.
    let mut sc = Some(db.decls[f].scope);
    while let Some(s) = sc {
        use yavs_sem::scope::ScopeKind as S;
        if matches!(db.scopes[s].kind, S::Process | S::Subprogram) {
            return Some(Err(format!(
                "`{shown}` is declared in a process or subprogram: conditions call functions of \
                 packages, entities, architectures and blocks only"
            )));
        }
        sc = db.scopes[s].parent;
    }
    let lowered = sh.calls.get(db, f);
    if let Err(e) = callable(sh, &lowered, &shown) {
        return Some(Err(e));
    }
    let Some(at) = at else {
        return Some(Ok(None));
    };
    Some(run_call(sh, site, f, &shown, args, at))
}

/// Whether the lowered function may be called: it lowered, and neither it nor anything it calls
/// writes (`Isolated::effect`).
fn callable(sh: &Shared, lowered: &Result<Isolated, String>, shown: &str) -> Result<(), String> {
    let l = match lowered {
        Ok(l) => l,
        Err(why) => {
            return Err(format!(
                "`{shown}` cannot be called in a condition: {}",
                plain_lowering(why)
            ));
        }
    };
    if let Some((what, span)) = l.effect() {
        let place = span
            .and_then(|s| crate::design::decl_of(&sh.db, s, &sh.link.source_map))
            .map_or(String::new(), |(f, n)| {
                let f = std::path::Path::new(&f)
                    .file_name()
                    .map_or(f.clone(), |f| f.to_string_lossy().into_owned());
                format!(" ({f}:{n})")
            });
        return Err(format!(
            "`{shown}` may write, so a condition does not call it: {what}{place}"
        ));
    }
    Ok(())
}

/// A lowering message about a function lowered on its own, in the words of a debugger's call:
/// the lowering speaks of a port conversion function, which reads signals the same way.
fn plain_lowering(why: &str) -> String {
    let signal = why
        .strip_prefix("a use of signal `")
        .and_then(|r| r.split_once("` other than its value in a port conversion function"));
    match signal {
        Some((name, _)) => format!(
            "it reads signal `{name}` other than by its value (an attribute, or as a signal \
             parameter), and a condition gives a function the values of signals only"
        ),
        None => match why
            .strip_prefix("object `")
            .and_then(|r| r.split_once("` declared outside the lowered units"))
        {
            Some((name, _)) => format!(
                "it reads `{name}`, which a condition cannot pass to it yet (a signal of a \
                 package, or a shared variable that is not protected)"
            ),
            None => why.to_string(),
        },
    }
}

fn run_call(
    sh: &Shared,
    site: &Site,
    f: DeclId,
    shown: &str,
    args: &[Arg<'_>],
    at: &At<'_, '_>,
) -> Result<Called, String> {
    let db = &*sh.db;
    let sp = db.decls[f]
        .kind
        .subprogram()
        .ok_or("this is a bug in yavs: no subprogram")?;
    let lowered = sh.calls.get(db, f);
    let Ok(l) = &*lowered else {
        return Err("this is a bug in yavs: a call that was not checked".into());
    };
    let now = match at.now() {
        Some(t) => t.as_fs(),
        None if l.reads_now() => {
            return Err(format!(
                "`{shown}` reads `now`, which has no value at this stop"
            ));
        }
        None => 0,
    };
    // ponytail: a constant that elaboration did not evaluate lives only in the instance data of
    // the running design; reading it there would need its offset in the process's instance.
    let mut sigs = HashMap::new();
    for (d, sig) in l.reads() {
        if sig {
            sigs.insert(
                d,
                signal_value(sh, site, d, at).map_err(|e| format!("`{shown}` {e}"))?,
            );
            continue;
        }
        if region_value(sh, site.inst, d).is_none() {
            return Err(format!(
                "`{shown}` reads `{}`, a constant whose value elaboration did not keep: \
                 conditions cannot call it yet",
                db.decls[d].name
            ));
        }
    }
    let mut vals = Vec::with_capacity(args.len());
    for (p, a) in sp.params.iter().zip(args) {
        let pt = db.base(p.subtype);
        let v = match a {
            Arg::Value(t, v, _) => to_val(db, pt, t, v),
            Arg::Literal(s) => read_literal(db, pt, s).and_then(|(t, v)| to_val(db, pt, &t, &v)),
            Arg::Unknown => None,
        };
        vals.push(
            v.ok_or_else(|| format!("`{shown}`: an argument of this type cannot be passed yet"))?,
        );
    }
    let read = |d: DeclId| match sigs.get(&d) {
        Some(v) => Some(v.clone()),
        None => region_value(sh, site.inst, d),
    };
    let v = l
        .call(&vals, &read, now)
        .map_err(|e| format!("`{shown}`: {e}"))?;
    let ret = sp
        .ret
        .map(|r| db.base(r))
        .ok_or("this is a bug in yavs: a function without a result")?;
    let (t, x) = from_val(db, ret, &v)
        .ok_or_else(|| format!("`{shown}`: its result cannot be shown yet"))?;
    Ok(Some((t, x, Some(type_name(db, ret)))))
}

/// The current value of signal `d` of a region around the function, seen from the process of
/// `site`, at stop `at`. The error follows the function's name.
fn signal_value(sh: &Shared, site: &Site, d: DeclId, at: &At<'_, '_>) -> Result<Val, String> {
    let db = &*sh.db;
    let name = &db.decls[d].name;
    let k = site
        .inst
        .and_then(|i| sh.elab.signal(i, d))
        .and_then(|o| sh.design.by_obj.get(&o).copied())
        .ok_or_else(|| format!("reads signal `{name}`, which is not seen from here"))?;
    let s = &sh.design.sigs[k];
    let v = s
        .value(&|r| at.read(r))
        .ok_or_else(|| format!("reads signal `{name}`, which cannot be read at this stop"))?;
    let yavs_sem::decl::DeclKind::Object(o) = &db.decls[d].kind else {
        return Err(format!("reads `{name}`, which is no signal"));
    };
    to_val(db, db.base(o.subtype), &s.lit, &v)
        .ok_or_else(|| format!("reads signal `{name}`, whose type cannot be passed yet"))
}

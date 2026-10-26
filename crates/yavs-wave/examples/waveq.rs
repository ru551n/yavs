//! Compare two waveform files (FST, VCD or GHW) signal by signal: tools/waveq/AGENTS.md.
//!
//! `cargo run --release -p yavs-wave --example waveq -- A B [--json OUT] [--skip GLOB]... [--context N]`
//! `cargo run --release -p yavs-wave --example waveq -- --list FILE`
//!
//! Names are matched after normalisation (`norm`), values at equal simulation times after the
//! last delta of each time step (`changes`). Exit 0 when every matched signal is equal, 1 when
//! one differs, 2 on a usage or read error.

use std::collections::BTreeMap;
use std::fmt::Write as _;

use wellen::simple::{Waveform, read};
use wellen::{Hierarchy, SignalValueRef, VarRef, VarType};

/// One signal: normalised name -> (display name, var).
type Names = BTreeMap<String, (String, VarRef)>;

fn main() {
    std::process::exit(match run() {
        Ok(c) => c,
        Err(e) => {
            eprintln!("waveq: {e}");
            2
        }
    });
}

fn run() -> Result<i32, String> {
    let mut args = std::env::args().skip(1);
    let (mut files, mut json, mut skip, mut context, mut list, mut until) =
        (vec![], None, vec![], 3, false, None::<u64>);
    while let Some(a) = args.next() {
        let mut val = |n: &str| args.next().ok_or(format!("{n} needs a value"));
        match a.as_str() {
            "--json" => json = Some(val("--json")?),
            "--skip" => skip.push(val("--skip")?.to_lowercase()),
            "--context" => {
                context = val("--context")?
                    .parse()
                    .map_err(|e| format!("--context: {e}"))?
            }
            "--until" => {
                until = Some(
                    val("--until")?
                        .parse()
                        .map_err(|e| format!("--until: {e}"))?,
                )
            }
            "--list" => list = true,
            _ if a.starts_with("--") => return Err(format!("unknown option {a}")),
            _ => files.push(a),
        }
    }
    let open = |f: &String| {
        if !std::path::Path::new(f).is_file() {
            return Err(format!("{f}: no such file"));
        }
        read(f).map_err(|e| format!("cannot read {f}: {e}"))
    };
    if list {
        let [f] = files.as_slice() else {
            return Err("--list takes one file".into());
        };
        let mut w = open(f)?;
        let names = names(w.hierarchy(), &[]);
        for (k, (full, v)) in &names {
            let ch = load(&mut w, [*v]).remove(0);
            let h = w.hierarchy();
            let var = &h[*v];
            println!(
                "{k}  [{full}] {:?} len={:?} enum={:?} type={:?} changes={} first={:?}",
                var.var_type(),
                var.length(h),
                var.enum_type(h).map(|e| e.0),
                var.vhdl_type_name(h),
                ch.len(),
                ch.iter().take(4).collect::<Vec<_>>()
            );
        }
        return Ok(0);
    }
    let [fa, fb] = files.as_slice() else {
        return Err("give two waveform files (or --list FILE)".into());
    };
    let (mut wa, mut wb) = (open(fa)?, open(fb)?);
    let (na, nb) = (names(wa.hierarchy(), &skip), names(wb.hierarchy(), &skip));
    let (ea, eb) = (end(&wa), end(&wb));
    let horizon = until.unwrap_or(ea.max(eb));
    let (pairs, only_a, only_b) = pair(&na, &nb);
    let mut rep = Report {
        only_a,
        only_b,
        ..Report::default()
    };
    // ponytail: signals load in chunks of 1000 per file; a design of millions of names would want
    // a streaming reader instead
    for chunk in pairs.chunks(1000) {
        let ca = load(&mut wa, chunk.iter().map(|p| p.1));
        let cb = load(&mut wb, chunk.iter().map(|p| p.2));
        for ((name, _, _), (ca, cb)) in chunk.iter().zip(ca.into_iter().zip(cb)) {
            let (ca, cb) = (upto(ca, horizon), upto(cb, horizon));
            if let (Some(x), Some(y)) = (ca.first(), cb.first())
                && logic(&x.1)
                && logic(&y.1)
                && x.1.len() != y.1.len()
            {
                // a port one writer records as its whole actual: equal when the shorter is one
                // fixed slice of the longer at every time
                let (short, long) = if x.1.len() < y.1.len() {
                    (&ca, &cb)
                } else {
                    (&cb, &ca)
                };
                let (n, m) = (short[0].1.len(), long[0].1.len());
                let fits = (0..=m - n).any(|o| {
                    let cut: Vec<(u64, String)> = long
                        .iter()
                        .map(|(t, v)| (*t, v.get(o..o + n).unwrap_or(v).to_string()))
                        .collect();
                    first_diff(short, &dedup(cut)).is_none()
                });
                if fits {
                    rep.sliced += 1;
                } else {
                    rep.shape
                        .push(format!("{name} ({} / {} elements)", x.1.len(), y.1.len()));
                }
                continue;
            }
            rep.signals += 1;
            rep.changes += ca.len().max(cb.len());
            match first_diff(&ca, &cb) {
                None => rep.equal += 1,
                Some(t) => rep.diffs.push(Diff {
                    name: name.clone(),
                    time: t,
                    a: around(&ca, t, context),
                    b: around(&cb, t, context),
                }),
            }
        }
    }
    print!("{}", rep.text(fa, fb, ea, eb));
    if let Some(j) = json {
        std::fs::write(&j, rep.json(fa, fb, ea, eb, horizon))
            .map_err(|e| format!("cannot write {j}: {e}"))?;
    }
    Ok(i32::from(!rep.diffs.is_empty()))
}

/// Matched (name in A, var in A, var in B), then the names only in A and only in B. A name that
/// does not match as is matches when one hierarchy level dropped from either side makes it equal
/// (a case-generate or library scope one writer leaves out).
#[allow(clippy::type_complexity)]
fn pair(na: &Names, nb: &Names) -> (Vec<(String, VarRef, VarRef)>, Vec<String>, Vec<String>) {
    let mut pairs = vec![];
    let mut rest_b: BTreeMap<&str, &(String, VarRef)> = nb
        .iter()
        .filter(|(k, _)| !na.contains_key(*k))
        .map(|(k, v)| (k.as_str(), v))
        .collect();
    let mut by_drop: BTreeMap<String, &str> = BTreeMap::new();
    for k in rest_b.keys() {
        for d in drops(k) {
            by_drop.entry(d).or_insert(k);
        }
    }
    let mut only_a = vec![];
    for (k, (full, va)) in na {
        let kb = if nb.contains_key(k) {
            Some(k.as_str())
        } else {
            by_drop.get(k).copied().or_else(|| {
                drops(k)
                    .into_iter()
                    .find_map(|d| rest_b.get_key_value(d.as_str()).map(|(kb, _)| *kb))
            })
        };
        match kb.and_then(|kb| nb.get(kb).map(|v| (kb, v))) {
            Some((kb, (_, vb))) if kb == k || rest_b.remove(kb).is_some() => {
                pairs.push((full.clone(), *va, *vb))
            }
            _ => only_a.push(full.clone()),
        }
    }
    (
        pairs,
        only_a,
        rest_b.values().map(|(f, _)| f.clone()).collect(),
    )
}

/// `k` with one of its scope levels left out.
fn drops(k: &str) -> Vec<String> {
    let segs: Vec<&str> = k.split('.').collect();
    (0..segs.len().saturating_sub(1))
        .map(|i| {
            segs.iter()
                .enumerate()
                .filter(|(j, _)| *j != i)
                .map(|(_, s)| *s)
                .collect::<Vec<_>>()
                .join(".")
        })
        .collect()
}

/// Normalised name of every variable not matched by a `skip` glob.
fn names(h: &Hierarchy, skip: &[String]) -> Names {
    let mut m = Names::new();
    for v in h.all_vars() {
        let full = h[v].full_name(h);
        let k = norm(&full);
        if !skip.iter().any(|g| glob(g, &k)) {
            m.entry(k).or_insert((full, v));
        }
    }
    m
}

/// Lower case; `(i)` and `[i]` both `[i]`; an element scope `.[i]` and `a.a[i]` both `a[i]`; a
/// trailing `[msb:lsb]` range dropped.
fn norm(s: &str) -> String {
    let mut s = s
        .to_lowercase()
        .replace('(', "[")
        .replace(')', "]")
        .replace(".[", "[");
    if s.ends_with(']')
        && let Some(i) = s.rfind('[')
        && s[i..].contains(':')
    {
        s.truncate(i);
    }
    let segs: Vec<&str> = s.split('.').collect();
    let mut out: Vec<&str> = vec![];
    for seg in segs {
        let base = seg.split('[').next().unwrap_or(seg);
        if seg.contains('[') && out.last().is_some_and(|l| *l == base) {
            out.pop();
        }
        out.push(seg);
    }
    out.join(".")
}

/// `*` any run of characters, `?` one character.
fn glob(p: &str, s: &str) -> bool {
    let (p, s) = (p.as_bytes(), s.as_bytes());
    let (mut i, mut j, mut star, mut mark) = (0, 0, None, 0);
    while j < s.len() {
        if i < p.len() && (p[i] == b'?' || p[i] == s[j]) {
            i += 1;
            j += 1;
        } else if i < p.len() && p[i] == b'*' {
            star = Some(i);
            mark = j;
            i += 1;
        } else if let Some(st) = star {
            i = st + 1;
            mark += 1;
            j = mark;
        } else {
            return false;
        }
    }
    p[i..].iter().all(|&c| c == b'*')
}

/// Last time of the file in femtoseconds.
fn end(w: &Waveform) -> u64 {
    w.time_table().last().map_or(0, |&t| t * fs(w))
}

fn fs(w: &Waveform) -> u64 {
    let ts = w.hierarchy().timescale();
    ts.map_or(1, |t| {
        let e = t.unit.to_exponent().unwrap_or(-15) + 15;
        u64::from(t.factor) * 10u64.pow(e as u32)
    })
}

/// Per var: (time in fs, value) after the last delta of each time step, repeats of a value
/// dropped.
fn load(w: &mut Waveform, vars: impl IntoIterator<Item = VarRef>) -> Vec<Vec<(u64, String)>> {
    let info: Vec<_> = {
        let h = w.hierarchy();
        vars.into_iter()
            .map(|v| {
                let var = &h[v];
                let e = var.enum_type(h).map(|(_, lits)| {
                    lits.into_iter()
                        .map(|(a, b)| (a.to_string(), b.to_lowercase().replace('\\', "")))
                        .collect::<Vec<_>>()
                });
                let kind = match var.var_type() {
                    _ if var
                        .vhdl_type_name(h)
                        .is_some_and(|n| n.eq_ignore_ascii_case("time")) =>
                    {
                        Kind::Time
                    }
                    VarType::Time => Kind::Time,
                    VarType::Integer => Kind::Int,
                    VarType::Boolean => Kind::Bool,
                    _ => Kind::Plain,
                };
                (var.signal_ref(), e, kind)
            })
            .collect()
    };
    let srs: Vec<_> = info.iter().map(|i| i.0).collect();
    w.load_signals_multi_threaded(&srs);
    let scale = fs(w);
    let tt = w.time_table().to_vec();
    let out = info
        .iter()
        .map(|(sr, enums, kind)| {
            let mut out: Vec<(u64, String)> = vec![];
            for (ti, val) in w.get_signal(*sr).into_iter().flat_map(|s| s.iter_changes()) {
                let t = tt[ti as usize] * scale;
                let s = value(val, enums.as_deref(), *kind);
                if out.last().is_some_and(|(lt, _)| *lt == t) {
                    out.pop();
                }
                if out.last().is_none_or(|(_, lv)| *lv != s) {
                    out.push((t, s));
                }
            }
            out
        })
        .collect();
    w.unload_signals(&srs);
    out
}

#[derive(Clone, Copy)]
enum Kind {
    Plain,
    Int,
    Time,
    Bool,
}

/// One value as text: enumeration literals by name, booleans as `false`/`true`, times in fs and
/// integers of 32 bits or more in decimal. A narrower integer stays `#bits` (`same`).
fn value(v: SignalValueRef, enums: Option<&[(String, String)]>, kind: Kind) -> String {
    let bits = match v {
        SignalValueRef::Real(r) => return format!("{r:e}"),
        // a writer that records TIME as text (`5 ns`), whatever its subtype is called
        SignalValueRef::String(s) => {
            return time_fs(s).unwrap_or_else(|| s.to_lowercase().replace('\\', ""));
        }
        SignalValueRef::Event => return "event".into(),
        SignalValueRef::BitVec(_) => v.to_bit_string().expect("bit vector"),
    };
    if let Some((_, name)) = enums.and_then(|e| e.iter().find(|(code, _)| *code == bits)) {
        return name.clone();
    }
    match kind {
        Kind::Bool if bits == "0" => "false".into(),
        Kind::Bool if bits == "1" => "true".into(),
        Kind::Int | Kind::Time if bits.len() >= 32 => signed(&bits).map_or(bits, |n| n.to_string()),
        Kind::Int => format!("#{bits}"),
        _ => bits,
    }
}

fn signed(bits: &str) -> Option<i128> {
    let u = u128::from_str_radix(bits, 2).ok()?;
    let w = bits.len() as u32;
    Some(if bits.starts_with('1') {
        u as i128 - (1i128 << w)
    } else {
        u as i128
    })
}

/// `1 ns` -> femtoseconds.
fn time_fs(s: &str) -> Option<String> {
    let (n, u) = s.trim().split_once(' ')?;
    let e = ["fs", "ps", "ns", "us", "ms", "sec", "min", "hr"]
        .iter()
        .position(|x| x.eq_ignore_ascii_case(u))?;
    let mul: i128 = [
        1,
        1_000,
        1_000_000,
        1_000_000_000,
        1_000_000_000_000,
        1_000_000_000_000_000,
        60_000_000_000_000_000,
        3_600_000_000_000_000_000,
    ][e];
    Some((n.parse::<i128>().ok()? * mul).to_string())
}

/// Equal values; an integer narrower than 32 bits (`#bits`, its signedness unknown to the
/// reader) equals a decimal that is its value read signed or unsigned.
fn same(a: &str, b: &str) -> bool {
    let num = |x: &str| -> Vec<i128> {
        match x.strip_prefix('#') {
            Some(bits) => [
                signed(bits),
                u128::from_str_radix(bits, 2).ok().map(|u| u as i128),
            ]
            .into_iter()
            .flatten()
            .collect(),
            None => x.parse().into_iter().collect(),
        }
    };
    a == b
        || ((a.starts_with('#') || b.starts_with('#')) && num(a).iter().any(|x| num(b).contains(x)))
}

fn upto(mut c: Vec<(u64, String)>, horizon: u64) -> Vec<(u64, String)> {
    c.retain(|(t, _)| *t <= horizon);
    c
}

/// First time at which the two values differ.
fn first_diff(a: &[(u64, String)], b: &[(u64, String)]) -> Option<u64> {
    let (mut i, mut j) = (0, 0);
    let (mut va, mut vb): (Option<&str>, Option<&str>) = (None, None);
    while i < a.len() || j < b.len() {
        let t = match (a.get(i), b.get(j)) {
            (Some(x), Some(y)) => x.0.min(y.0),
            (Some(x), None) => x.0,
            (None, Some(y)) => y.0,
            (None, None) => unreachable!(),
        };
        while a.get(i).is_some_and(|x| x.0 == t) {
            va = Some(&a[i].1);
            i += 1;
        }
        while b.get(j).is_some_and(|y| y.0 == t) {
            vb = Some(&b[j].1);
            j += 1;
        }
        if va.zip(vb).is_none_or(|(x, y)| !same(x, y)) && va != vb {
            return Some(t);
        }
    }
    None
}

/// Consecutive equal values merged.
fn dedup(mut c: Vec<(u64, String)>) -> Vec<(u64, String)> {
    c.dedup_by(|b, a| a.1 == b.1);
    c
}

/// A logic vector or scalar (`0`, `1`, `u`, ...) rather than a number or a name.
fn logic(v: &str) -> bool {
    !v.is_empty() && v.bytes().all(|c| b"01xzuwlh-".contains(&c))
}

/// Up to `n` changes before `t` and `n` from `t` on.
fn around(c: &[(u64, String)], t: u64, n: usize) -> Vec<(u64, String)> {
    let k = c.partition_point(|(x, _)| *x < t);
    c[k.saturating_sub(n)..(k + n).min(c.len())].to_vec()
}

struct Diff {
    name: String,
    time: u64,
    a: Vec<(u64, String)>,
    b: Vec<(u64, String)>,
}

#[derive(Default)]
struct Report {
    signals: usize,
    changes: usize,
    equal: usize,
    diffs: Vec<Diff>,
    only_a: Vec<String>,
    only_b: Vec<String>,
    /// Matched names whose vectors have different lengths (a port one writer records as its
    /// whole actual): equal as a fixed slice (`sliced`), or not (`shape`).
    sliced: usize,
    shape: Vec<String>,
}

impl Report {
    fn text(&self, fa: &str, fb: &str, ea: u64, eb: u64) -> String {
        let mut s = String::new();
        let _ = writeln!(s, "A {fa} (end {ea} fs)\nB {fb} (end {eb} fs)");
        let _ = writeln!(
            s,
            "signals {} changes {} equal {} different {} only-A {} only-B {} sliced {} shape {}",
            self.signals,
            self.changes,
            self.equal,
            self.diffs.len(),
            self.only_a.len(),
            self.only_b.len(),
            self.sliced,
            self.shape.len()
        );
        for d in &self.diffs {
            let _ = writeln!(
                s,
                "DIFF {} at {} fs\n  A {:?}\n  B {:?}",
                d.name, d.time, d.a, d.b
            );
        }
        for n in &self.only_a {
            let _ = writeln!(s, "ONLY-A {n}");
        }
        for n in &self.only_b {
            let _ = writeln!(s, "ONLY-B {n}");
        }
        for n in &self.shape {
            let _ = writeln!(s, "SHAPE {n}");
        }
        s
    }

    fn json(&self, fa: &str, fb: &str, ea: u64, eb: u64, until: u64) -> String {
        let q = |s: &str| format!("\"{}\"", s.replace('\\', "\\\\").replace('"', "\\\""));
        let ch = |c: &[(u64, String)]| {
            let v: Vec<String> = c.iter().map(|(t, x)| format!("[{t},{}]", q(x))).collect();
            format!("[{}]", v.join(","))
        };
        let list =
            |v: &[String]| format!("[{}]", v.iter().map(|x| q(x)).collect::<Vec<_>>().join(","));
        let diffs: Vec<String> = self
            .diffs
            .iter()
            .map(|d| {
                format!(
                    "{{\"name\":{},\"time_fs\":{},\"a\":{},\"b\":{}}}",
                    q(&d.name),
                    d.time,
                    ch(&d.a),
                    ch(&d.b)
                )
            })
            .collect();
        format!(
            "{{\"a\":{},\"b\":{},\"end_a_fs\":{ea},\"end_b_fs\":{eb},\"until_fs\":{until},\"signals\":{},\"changes\":{},\"equal\":{},\"different\":{},\"diffs\":[{}],\"only_a\":{},\"only_b\":{},\"sliced\":{},\"shape\":{}}}\n",
            q(fa),
            q(fb),
            self.signals,
            self.changes,
            self.equal,
            self.diffs.len(),
            diffs.join(","),
            list(&self.only_a),
            list(&self.only_b),
            self.sliced,
            list(&self.shape)
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ch(v: &[(u64, &str)]) -> Vec<(u64, String)> {
        v.iter().map(|(t, s)| (*t, s.to_string())).collect()
    }

    #[test]
    fn names_normalise() {
        assert_eq!(norm("Top.G(0).U.Q[3:0]"), "top.g[0].u.q");
        assert_eq!(norm("t.ds.(1)"), norm("t.ds.[1]"));
        assert_eq!(norm("t2.ar.ar(0).v"), "t2.ar[0].v");
        assert!(drops("t.gc.one.cs").contains(&"t.one.cs".to_string()));
        assert!(glob("top.*.mem*", "top.u1.mem[3]") && !glob("top.u?", "top.u12"));
    }

    #[test]
    fn values() {
        assert!(same("#11111", "31") && same("#1101", "-3") && !same("#1101", "12x"));
        assert!(!same("0", "1") && same("u", "u"));
        assert_eq!(time_fs("3 ns").as_deref(), Some("3000000"));
        assert_eq!(signed(&"1".repeat(32)), Some(-1));
    }

    #[test]
    fn first_difference() {
        let a = ch(&[(0, "u"), (5, "0"), (10, "1")]);
        assert_eq!(first_diff(&a, &a), None);
        assert_eq!(
            first_diff(&a, &ch(&[(0, "u"), (5, "0"), (12, "1")])),
            Some(10)
        );
        assert_eq!(first_diff(&a, &ch(&[(0, "u"), (5, "0")])), Some(10));
        assert_eq!(first_diff(&ch(&[(0, "#11111")]), &ch(&[(0, "31")])), None);
    }
}

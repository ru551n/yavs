//! What the key of a process's copies must hold (`copies.rs`). A value left out of the key is a
//! hole: the copy reads it from the running instance, as it does the parameter of a `for`
//! generate, and instances that differ in nothing else share one body. Leaving a value out is
//! always correct and costs what folding it would have saved, so this decides by what the lowered
//! code does with a scalar: nothing (no cost), arithmetic and branches (`Value`: the facts stay
//! known but for that value), or a size, a bound or a descriptor (`Shape`, which stays in the key).

use std::collections::HashSet;
use std::sync::{Arc, OnceLock};

use crate::{AddrSpace, Func, Inst, Program, Reg, Role};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Mode {
    /// Every byte in the key, but the parameters of `for` generates.
    Off,
    /// Also out: scalars the process does not read.
    Unread,
    /// Also out: scalars it only computes with, when instances differ in them and that merges
    /// bodies.
    Value,
    /// Test aid: every scalar it only computes with, so that a run of one instance reads them.
    All,
}

/// `YAVS_SHARE_KEY`: `off`, `unread`, `value` or `all`.
pub(crate) fn mode() -> Mode {
    static MODE: OnceLock<Mode> = OnceLock::new();
    *MODE.get_or_init(|| match std::env::var("YAVS_SHARE_KEY").as_deref() {
        Ok("off") => Mode::Off,
        Ok("unread") => Mode::Unread,
        Ok("all") => Mode::All,
        _ => Mode::Value,
    })
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Use {
    Unread,
    Value,
    Shape,
}

/// The registers an instruction reads as a size, a length, a bound or a descriptor: what the
/// fold of a value would turn into a different form of the code.
fn shape_operands(i: &Inst, ops: &[Reg], out: &mut Vec<Reg>) {
    let list = |r: crate::Regs| {
        ops.get(r.start as usize..(r.start + r.len) as usize)
            .unwrap_or(&[])
    };
    match *i {
        Inst::Alloc { size, .. } | Inst::PkgAlloc { size, .. } => out.push(size),
        Inst::Copy { bytes, .. } => out.push(bytes),
        Inst::Fill { n, .. }
        | Inst::LogicOp { n, .. }
        | Inst::LNum { n, .. }
        | Inst::LEq { n, .. }
        | Inst::LKey { n, .. }
        | Inst::MemEq { n, .. }
        | Inst::LFill { n, .. }
        | Inst::LCopy { n, .. }
        | Inst::Drive { n, .. }
        | Inst::DriveSlot { n, .. }
        | Inst::SigParts { n, .. } => out.push(n),
        Inst::Pos { dope, .. } | Inst::DopeLen { dope, .. } => out.push(dope),
        Inst::SlicePos {
            dope,
            left,
            right,
            dir,
            ..
        } => out.extend([dope, left, right, dir]),
        Inst::Field { layout, .. } => out.push(layout),
        Inst::Cat { n, m, parts, .. } => {
            out.extend([n, m]);
            out.extend_from_slice(list(parts));
        }
        Inst::MakeDope { at, parts } => {
            out.push(at);
            out.extend_from_slice(list(parts));
        }
        Inst::LayoutOf { desc, .. } | Inst::SigGather { desc, .. } => out.push(desc),
        Inst::SigScatter { desc, .. } => out.push(desc),
        Inst::SubtypeCheck { dope, want, .. } => out.extend([dope, want]),
        Inst::New { size, .. } => out.push(size),
        Inst::NewFill { desc, size, .. } | Inst::NewFrame { desc, size, .. } => {
            out.extend([desc, size]);
        }
        Inst::TempRoot { desc: Some(d), .. } => out.push(d),
        // (Where in the signal is a value, as the parameter of a generate is.)
        Inst::SigVal { len, .. } | Inst::SigSlice { len, .. } => out.push(len),
        _ => {}
    }
}

/// The registers of a native call that size its result (`opt.rs` `native_len` folds them).
fn native_sizes(prog: &Program, native: crate::NativeId, args: &[Reg], out: &mut Vec<Reg>) {
    use crate::ieee::{ARRAY_REGS, Op, array_param, parse, symbol_key};
    let Some(key) = prog.natives.get(native).and_then(|n| symbol_key(&n.symbol)) else {
        return;
    };
    let Some(op) = parse(key) else {
        return;
    };
    let (mut p, mut i) = (0, 0);
    while p < args.len() {
        if array_param(key, i) {
            out.extend(args[p + 1..].iter().take(ARRAY_REGS - 1));
            p += ARRAY_REGS;
        } else {
            if i == 1 && matches!(op, Op::ToUnsigned | Op::ToSigned | Op::Resize { .. }) {
                out.push(args[p]);
            }
            p += 1;
        }
        i += 1;
    }
}

/// What the lowered code of a process does with the scalar in instance data bytes `o .. o + n`.
pub(crate) fn classify(prog: &Program, f: &Func, (o, n): (u32, u32)) -> Use {
    let nregs = f.reg_ty.len();
    let overlaps = |off: u32| off >= o && off < o + n;
    if !f
        .code
        .iter()
        .any(|i| matches!(i, Inst::AddrOf { space: AddrSpace::Inst, off, .. } if overlaps(*off)))
    {
        return Use::Unread;
    }
    // Per register: holds the scalar's address; holds something computed from the scalar; the
    // temporary block it points into, by the block's register; the frame object it points into.
    let (mut addr, mut val) = (vec![false; nregs], vec![false; nregs]);
    let mut root: Vec<Option<Reg>> = vec![None; nregs];
    let mut frame: Vec<Option<u32>> = vec![None; nregs];
    // The frame objects that hold something computed from the scalar.
    let mut held = HashSet::new();
    let at = |r: Reg| r.0 as usize;
    let mut sinks = Vec::new();
    let mut changed = true;
    while changed {
        changed = false;
        let mark = |v: &mut Vec<bool>, r: Reg, changed: &mut bool| {
            if !v[at(r)] {
                v[at(r)] = true;
                *changed = true;
            }
        };
        for i in &f.code {
            match *i {
                Inst::AddrOf {
                    dst,
                    space: AddrSpace::Inst,
                    off,
                } if overlaps(off) => {
                    if off != o {
                        return Use::Shape;
                    }
                    mark(&mut addr, dst, &mut changed);
                    continue;
                }
                Inst::Alloc { dst, .. } => {
                    if root[at(dst)].is_none() {
                        root[at(dst)] = Some(dst);
                        changed = true;
                    }
                }
                Inst::AddrOf {
                    dst,
                    space: AddrSpace::Frame,
                    off,
                } => {
                    if frame[at(dst)].is_none() {
                        frame[at(dst)] = Some(off);
                        changed = true;
                    }
                    if held.contains(&off) {
                        mark(&mut val, dst, &mut changed);
                    }
                }
                Inst::Move { dst, src } => {
                    if addr[at(src)] {
                        mark(&mut addr, dst, &mut changed);
                    }
                    if root[at(dst)].is_none() && root[at(src)].is_some() {
                        root[at(dst)] = root[at(src)];
                        changed = true;
                    }
                    if frame[at(dst)].is_none() && frame[at(src)].is_some() {
                        frame[at(dst)] = frame[at(src)];
                        changed = true;
                    }
                }
                Inst::Index { dst, base, .. } | Inst::Field { dst, base, .. } => {
                    if root[at(dst)].is_none() && root[at(base)].is_some() {
                        root[at(dst)] = root[at(base)];
                        changed = true;
                    }
                    if frame[at(dst)].is_none() && frame[at(base)].is_some() {
                        frame[at(dst)] = frame[at(base)];
                        changed = true;
                    }
                }
                Inst::Load { dst, addr: a, .. } if addr[at(a)] => {
                    mark(&mut val, dst, &mut changed);
                    continue;
                }
                Inst::Store { addr: a, src, .. } if val[at(src)] => {
                    // Into a temporary (a waveform, an aggregate) or a variable of the process
                    // the object holds the value; anywhere else it cannot be followed.
                    if let Some(b) = root[at(a)] {
                        mark(&mut val, b, &mut changed);
                    } else if let Some(off) = frame[at(a)] {
                        changed |= held.insert(off);
                    } else {
                        return Use::Shape;
                    }
                }
                Inst::Copy { dst, src, .. } if val[at(src)] => {
                    if let Some(b) = root[at(dst)] {
                        mark(&mut val, b, &mut changed);
                    } else if let Some(off) = frame[at(dst)] {
                        changed |= held.insert(off);
                    } else {
                        return Use::Shape;
                    }
                }
                _ => {}
            }
            // The address itself goes somewhere it cannot be followed.
            let mut escapes = false;
            let is_move = matches!(i, Inst::Move { .. });
            i.visit(&mut |r, role, _| {
                if role == Role::Use && addr[at(r)] && !is_move {
                    escapes = true;
                }
            });
            let list = i
                .list()
                .and_then(|r| f.operands.get(r.start as usize..(r.start + r.len) as usize))
                .unwrap_or(&[]);
            if escapes || list.iter().any(|r| addr[at(*r)]) {
                return Use::Shape;
            }
            sinks.clear();
            shape_operands(i, &f.operands, &mut sinks);
            if let Inst::CallNative { native, .. } = *i {
                native_sizes(prog, native, list, &mut sinks);
            }
            if sinks.iter().any(|r| val[at(*r)]) {
                return Use::Shape;
            }
            // The result of a call is not folded from its arguments: it is no less known.
            if matches!(
                i,
                Inst::Call { .. }
                    | Inst::CallNative { .. }
                    | Inst::CallForeign { .. }
                    | Inst::CallIndirect { .. }
                    | Inst::CallProt { .. }
            ) {
                continue;
            }
            let mut read = list.iter().any(|r| val[at(*r)]);
            i.visit(&mut |r, role, _| read |= role == Role::Use && val[at(r)]);
            if read {
                let mut defs = Vec::new();
                i.visit(&mut |r, role, _| {
                    if role == Role::Def {
                        defs.push(r);
                    }
                });
                for d in defs {
                    mark(&mut val, d, &mut changed);
                }
            }
        }
    }
    Use::Value
}

fn masked_hash(d: &[u8], holes: &[(u32, u32)]) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    let mut at = 0;
    for &(o, n) in holes {
        let (o, e) = (o as usize, o as usize + n as usize);
        if o >= at {
            d.get(at..o).hash(&mut h);
            at = e;
        }
    }
    d.get(at..).hash(&mut h);
    h.finish()
}

/// `YAVS_SHARE_MERGE`: values are held when that makes at most one body per so many.
fn merge_factor() -> usize {
    static N: OnceLock<usize> = OnceLock::new();
    *N.get_or_init(|| {
        std::env::var("YAVS_SHARE_MERGE")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(2)
    })
}

/// The holes every copy of process `f` gets, from the ranges `gen_holes` every copy has, the
/// scalars of its unit and the data of its instances, and the bodies its instances need then. A
/// value is held only where instances differ in it and that merges bodies: holding it costs its
/// folds in every instance.
pub(crate) fn plan(
    prog: &Program,
    f: &Func,
    gen_holes: &[(u32, u32)],
    scalars: &[(u32, u32)],
    insts: &[Arc<dyn AsRef<[u8]> + Send + Sync>],
    mode: Mode,
) -> (Vec<(u32, u32)>, usize) {
    let keys = |h: &[(u32, u32)]| {
        let mut h = h.to_vec();
        h.sort_unstable();
        let s: HashSet<u64> = insts
            .iter()
            .map(|d| masked_hash((**d).as_ref(), &h))
            .collect();
        s.len()
    };
    // Nothing to merge: every fold stays.
    if mode != Mode::All && keys(gen_holes) == 1 {
        return (gen_holes.to_vec(), 1);
    }
    let mut holes = gen_holes.to_vec();
    let mut value = Vec::new();
    // (A region holds the values of its unit up to its own end.)
    let within = |&(o, n): &(u32, u32)| {
        insts
            .iter()
            .all(|d| (**d).as_ref().len() >= (o + n) as usize)
    };
    for &s in scalars.iter().filter(|s| within(s)) {
        match classify(prog, f, s) {
            Use::Unread => holes.push(s),
            Use::Value => value.push(s),
            Use::Shape => {}
        }
    }
    if mode == Mode::All {
        holes.extend(value);
        holes.sort_unstable();
        let n = keys(&holes);
        return (holes, n);
    }
    let unread = keys(&holes);
    if mode == Mode::Value && unread > 1 {
        let bytes = |d: &Arc<dyn AsRef<[u8]> + Send + Sync>, (o, n): (u32, u32)| {
            (**d)
                .as_ref()
                .get(o as usize..(o + n) as usize)
                .map(<[u8]>::to_vec)
        };
        value.retain(|&s| insts.iter().any(|d| bytes(d, s) != bytes(&insts[0], s)));
        let all: Vec<_> = holes.iter().chain(&value).copied().collect();
        let held = keys(&all);
        if held < unread && held * merge_factor() <= unread {
            holes = all;
        }
        if std::env::var_os("YAVS_SHARE_CENSUS").is_some() {
            eprintln!(
                "sharekey {} code {} instances {} keys full {} unread {} value {} held {}",
                f.name,
                f.code.len(),
                insts.len(),
                keys(gen_holes),
                unread,
                held,
                value.len(),
            );
        }
    }
    holes.sort_unstable();
    let n = keys(&holes);
    (holes, n)
}

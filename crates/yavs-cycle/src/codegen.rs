//! Cranelift code of the region: one function per trigger, the operations of the member bodies in
//! schedule order on the flat image of two-state words. A logic value of `len` elements is
//! `ceil(len / 64)` words, element 0 in bit 0 of the first, the bits above `len` zero; an integer
//! is one word.
//!
//! A check that fails (`Op::Int`, `Op::Range`, the dynamic indices) stores its operands in the
//! two error words and returns its site number plus one; `RegionProc` makes the error. With
//! `YAVS_CYCLE_CHECKS=off` (developers, to measure them) range and overflow checks are left out
//! and a dynamic index is clamped into its vector, so the code stays memory safe.
//!
//! The only `unsafe` of the crate: the region calls the generated code on its image.
#![allow(unsafe_code)]

use std::collections::BTreeMap;

use cranelift_codegen::ir::condcodes::IntCC;
use cranelift_codegen::ir::types::{I32, I64};
use cranelift_codegen::ir::{AbiParam, Block as CBlock, InstBuilder, MemFlagsData, Value};
use cranelift_codegen::settings::{self, Configurable};
use cranelift_frontend::{FunctionBuilder, FunctionBuilderContext};
use cranelift_jit::{JITBuilder, JITModule};
use cranelift_module::{FuncId, Linkage, Module};

use crate::lower::{Arith, Cmp, Gate, Op, Ref, ShiftK};

/// One compiled trigger function: the address of the image's first word; 0, or the failed check's
/// site plus one.
pub(crate) type Entry = unsafe extern "C" fn(*mut u64) -> u32;

/// The compiled functions of a region; the module owns their code.
pub(crate) struct Code {
    _module: JITModule,
    /// By [`Fun`]: the functions it is cut into, called in order.
    entries: [Vec<Entry>; 3],
    /// The words the code addresses.
    words: usize,
}

impl Code {
    /// Run function `f` on `img`: `Err(site)` when a check failed.
    pub(crate) fn run(&self, img: &mut [u64], f: Fun) -> Result<(), u32> {
        assert!(
            img.len() >= self.words,
            "the image is smaller than the code's words"
        );
        for f in &self.entries[f as usize] {
            // SAFETY: the code reads and writes only words below `words`, checked above.
            match unsafe { f(img.as_mut_ptr()) } {
                0 => {}
                s => return Err(s - 1),
            }
        }
        Ok(())
    }
}

// SAFETY: the code is finished and never changed again; the module is only kept alive.
unsafe impl Send for Code {}
// SAFETY: as above, nothing in it is mutated after `compile`.
unsafe impl Sync for Code {}

/// The trigger functions of a region.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum Fun {
    /// The combinational bodies.
    Settle,
    /// The clocked bodies, the commit, the combinational bodies.
    Edge,
    /// Once, before the first settle: initial values of the variables.
    Init,
}

/// Ones in the low `len` bits.
pub(crate) fn mask(len: u32) -> u64 {
    if len >= 64 { !0 } else { (1u64 << len) - 1 }
}

/// Words of a logic value of `len` elements.
pub(crate) fn words(len: u32) -> u32 {
    len.div_ceil(64).max(1)
}

/// Whether range, overflow and index checks are compiled (`YAVS_CYCLE_CHECKS=off`: not).
pub(crate) fn checks_on() -> bool {
    std::env::var_os("YAVS_CYCLE_CHECKS").is_none_or(|v| v != "off")
}

fn word(r: Ref) -> i32 {
    match r {
        Ref::Loc(s) => (s * 8) as i32,
        Ref::Sig(_) | Ref::Edge => {
            unreachable!("signal references are resolved to slots before code generation")
        }
    }
}

/// Operations per compiled function, about: Cranelift's register allocation grows faster than
/// the function, so a long trigger function is cut between members into several.
const CHUNK: usize = 4000;

/// The operations of `ops`, nested ones included.
fn weight(ops: &[Op]) -> usize {
    ops.iter()
        .map(|o| match o {
            Op::If { then, else_, .. } => 1 + weight(then) + weight(else_),
            _ => 1,
        })
        .sum()
}

/// Compile the trigger functions (by [`Fun`]), each given as pieces that need nothing from one
/// another but memory (the bodies of members). Every `Ref` of them is an image slot
/// (`Ref::Loc`); `err` is the first of the two error words; `keep[w]`: word `w` holds state
/// between calls (signals, variables of clocked processes), every other word is scratch, which
/// the code keeps in registers and never stores unless an element of it is addressed at run
/// time. `name` labels the functions in the perf map.
pub(crate) fn compile(
    funs: [&[Vec<Op>]; 3],
    keep: &[bool],
    err: u32,
    name: &str,
) -> Result<Code, String> {
    let words = keep.len();
    let mut flags = settings::builder();
    let set = |f: &mut settings::Builder, k: &str, v: &str| f.set(k, v).map_err(|e| e.to_string());
    set(&mut flags, "opt_level", "speed")?;
    set(
        &mut flags,
        "enable_verifier",
        if cfg!(debug_assertions) {
            "true"
        } else {
            "false"
        },
    )?;
    let isa = cranelift_native::builder()
        .map_err(|e| e.to_string())?
        .finish(settings::Flags::new(flags))
        .map_err(|e| e.to_string())?;
    let jb = JITBuilder::with_isa(isa, cranelift_module::default_libcall_names());
    let mut module = JITModule::new(jb);
    let ptr = module.target_config().pointer_type();
    let checks = checks_on();
    let mut ids = Vec::new();
    let perf = std::env::var_os("YAVS_CLIFT_PERFMAP").is_some();
    let mut chunks: Vec<(usize, Vec<&Vec<Op>>)> = Vec::new();
    for (f, pieces) in funs.iter().enumerate() {
        let mut cur: Vec<&Vec<Op>> = Vec::new();
        let mut w = 0;
        for p in pieces.iter() {
            if w > 0 && w + weight(p) > CHUNK {
                chunks.push((f, std::mem::take(&mut cur)));
                w = 0;
            }
            w += weight(p);
            cur.push(p);
        }
        // Every function has one entry at least, maybe empty.
        if !cur.is_empty() || !chunks.iter().any(|c| c.0 == f) {
            chunks.push((f, cur));
        }
    }
    for (k, (f, ops)) in chunks.iter().enumerate() {
        let what = ["settle", "edge", "init"][*f];
        let sym = format!("cycle_{what}_{k}_{name}");
        let mut sig = module.make_signature();
        sig.params.push(AbiParam::new(ptr));
        sig.returns.push(AbiParam::new(I32));
        let id = module
            .declare_function(&sym, Linkage::Local, &sig)
            .map_err(|e| e.to_string())?;
        let mut ctx = module.make_context();
        ctx.func.signature = sig;
        let mut fbc = FunctionBuilderContext::new();
        {
            let mut b = FunctionBuilder::new(&mut ctx.func, &mut fbc);
            let blk = b.create_block();
            b.append_block_params_for_function_params(blk);
            b.switch_to_block(blk);
            b.seal_block(blk);
            let base = b.block_params(blk)[0];
            let mut g = Gen {
                b,
                base,
                err: err as i32 * 8,
                checks,
                ptr,
                cache: BTreeMap::new(),
                keep,
                zeros: std::collections::HashMap::new(),
            };
            // Each piece starts from memory: the words it keeps in registers are few, and a
            // branch saves and forgets only those.
            for p in ops {
                for op in p.iter() {
                    g.op(op);
                }
                g.write_back();
                g.cache.clear();
                g.zeros.clear();
            }
            let zero = g.b.ins().iconst(I32, 0);
            g.b.ins().return_(&[zero]);
            g.b.finalize(module.target_config());
        }
        // Developers: `YAVS_CYCLE_DISASM=1` prints the machine code of each function.
        let disasm = std::env::var_os("YAVS_CYCLE_DISASM").is_some();
        ctx.set_disasm(disasm);
        module
            .define_function(id, &mut ctx)
            .map_err(|e| format!("{e:?}"))?;
        if disasm && let Some(text) = ctx.compiled_code().and_then(|c| c.vcode.as_ref()) {
            eprintln!("{sym}:\n{text}");
        }
        ids.push((*f, id, sym));
    }
    module.finalize_definitions().map_err(|e| e.to_string())?;
    let addr = |id: FuncId| module.get_finalized_function(id);
    if perf {
        for (_, id, sym) in &ids {
            // The size is not kept by the module; one page is an upper bound for a label.
            yavs_rt::perfmap::append(addr(*id) as usize, 4096, sym);
        }
    }
    let mut entries: [Vec<Entry>; 3] = Default::default();
    for (f, id, _) in &ids {
        // SAFETY: every one is a function of signature `fn(ptr) -> u32` defined above.
        entries[*f].push(unsafe { std::mem::transmute::<*const u8, Entry>(addr(*id)) });
    }
    Ok(Code {
        _module: module,
        entries,
        words,
    })
}

struct Gen<'a> {
    b: FunctionBuilder<'a>,
    base: Value,
    /// Byte offset of the first error word.
    err: i32,
    checks: bool,
    ptr: cranelift_codegen::ir::Type,
    /// The words in registers, by byte offset: their value and whether memory is behind it. Every
    /// value dominates the code that follows (a branch drops what it made at its join).
    cache: BTreeMap<i32, (Value, bool)>,
    keep: &'a [bool],
    /// Bits known to be zero of some values (constants, extracted bits, words built by inserts).
    zeros: std::collections::HashMap<Value, u64>,
}

impl Gen<'_> {
    fn ops(&mut self, ops: &[Op]) {
        for op in ops {
            self.op(op);
        }
    }

    /// Word `k` of `r`: from a register when it has one.
    fn ld(&mut self, r: Ref, k: u32) -> Value {
        let off = word(r) + k as i32 * 8;
        if let Some(&(v, _)) = self.cache.get(&off) {
            return v;
        }
        let v = self
            .b
            .ins()
            .load(I64, MemFlagsData::trusted(), self.base, off);
        self.cache.insert(off, (v, false));
        v
    }

    /// Word `k` of `r` is `v` from here on; memory follows at `write_back` or `spill`.
    fn st(&mut self, r: Ref, k: u32, v: Value) {
        self.cache.insert(word(r) + k as i32 * 8, (v, true));
    }

    fn store(&mut self, off: i32, v: Value) {
        self.b
            .ins()
            .store(MemFlagsData::trusted(), v, self.base, off);
    }

    /// The words that hold state, into memory (the end of the function).
    fn write_back(&mut self) {
        let dirty: Vec<(i32, Value)> = (self.cache.iter())
            .filter(|(off, (_, d))| *d && self.keep[**off as usize / 8])
            .map(|(&off, &(v, _))| (off, v))
            .collect();
        for (off, v) in dirty {
            self.store(off, v);
        }
    }

    /// Words `k0 ..= k1` of `r` into memory, scratch ones too: code is about to address one of
    /// their elements at run time.
    fn spill(&mut self, r: Ref, k0: u32, k1: u32) {
        for k in k0..=k1 {
            let off = word(r) + k as i32 * 8;
            if let Some((v, d)) = self.cache.get_mut(&off)
                && *d
            {
                *d = false;
                let v = *v;
                self.store(off, v);
            }
        }
    }

    /// Words `k0 ..= k1` of `r` changed in memory.
    fn forget(&mut self, r: Ref, k0: u32, k1: u32) {
        for k in k0..=k1 {
            self.cache.remove(&(word(r) + k as i32 * 8));
        }
    }

    fn lds(&mut self, r: Ref, len: u32) -> Vec<Value> {
        (0..words(len)).map(|k| self.ld(r, k)).collect()
    }

    fn sts(&mut self, r: Ref, v: &[Value]) {
        for (k, &x) in v.iter().enumerate() {
            self.st(r, k as u32, x);
        }
    }

    fn konst(&mut self, v: u64) -> Value {
        let x = self.b.ins().iconst(I64, v as i64);
        self.zeros.insert(x, !v);
        x
    }

    /// The top word of a value of `len` elements with the bits above `len` cleared.
    fn top(&mut self, v: &mut [Value], len: u32) {
        if !len.is_multiple_of(64)
            && let Some(t) = v.last_mut()
        {
            *t = self.b.ins().band_imm_s(*t, mask(len % 64) as i64);
        }
    }

    /// `n` (at most 64) bits of the words `v` from bit `off`, in the low bits; `full`: the bits
    /// of `v` from `full` on are zero, so no mask is needed past them.
    fn bits(&mut self, v: &[Value], off: u32, n: u32, full: u32) -> Value {
        let (k, s) = ((off / 64) as usize, off % 64);
        let mut x = v[k];
        let mut end = 64 * (k as u32 + 1);
        if s != 0 {
            x = self.b.ins().ushr_imm_s(x, i64::from(s));
            if s + n > 64 {
                let h = self.b.ins().ishl_imm_s(v[k + 1], i64::from(64 - s));
                x = self.b.ins().bor(x, h);
                end = off + 64;
            }
        }
        if n < 64 && off + n < full.min(end) {
            x = self.b.ins().band_imm_s(x, mask(n) as i64);
        }
        if n < 64 {
            self.zeros.insert(x, !mask(n));
        }
        x
    }

    /// The bits of `v` known to be zero.
    fn zero(&self, v: Value) -> u64 {
        self.zeros.get(&v).copied().unwrap_or(0)
    }

    /// `d` with bits `lo .. lo + len` taken from `s` (`len` elements).
    fn insert(&mut self, d: &mut [Value], s: &[Value], lo: u32, len: u32) {
        let mut a = lo;
        while a < lo + len {
            let k = a / 64;
            let b = (lo + len).min(k * 64 + 64);
            let x = self.bits(s, a - lo, b - a, len);
            let sh = a - k * 64;
            let m = mask(b - a) << sh;
            let dk = d[k as usize];
            let zd = self.zero(dk);
            // Bits known to be zero need no clearing (a word built field by field from 0).
            let keep = if zd & m == m {
                dk
            } else {
                self.b.ins().band_imm_s(dk, !m as i64)
            };
            let x = if sh != 0 {
                self.b.ins().ishl_imm_s(x, i64::from(sh))
            } else {
                x
            };
            let r = self.b.ins().bor(keep, x);
            let zx = self.zero(x) << sh | mask(sh);
            self.zeros.insert(r, (zd | m) & (zx | !m));
            d[k as usize] = r;
            a = b;
        }
    }

    fn bool_of(&mut self, c: Value) -> Value {
        self.b.ins().uextend(I64, c)
    }

    /// A check: `fail` (an `I8` truth value) jumps to a cold block that stores `vals` in the
    /// error words and returns `site + 1`.
    fn check(&mut self, fail: Value, site: u32, vals: &[Value]) {
        let bad = self.b.create_block();
        let ok = self.b.create_block();
        self.b.set_cold_block(bad);
        self.b.ins().brif(fail, bad, &[], ok, &[]);
        self.b.switch_to_block(bad);
        self.b.seal_block(bad);
        for (k, &v) in vals.iter().enumerate() {
            self.b.ins().store(
                MemFlagsData::trusted(),
                v,
                self.base,
                self.err + k as i32 * 8,
            );
        }
        let s = self.b.ins().iconst(I32, i64::from(site) + 1);
        self.b.ins().return_(&[s]);
        self.b.switch_to_block(ok);
        self.b.seal_block(ok);
    }

    /// The element of a dynamic index: `idx - left` (ascending) or `left - idx`, checked to be
    /// below `n` and, for `count` elements from it to the left, at least `count - 1` (or clamped
    /// when checks are off); returns its first bit, `lo + (n - 1 - p) * w`.
    #[allow(clippy::too_many_arguments)]
    fn element(
        &mut self,
        idx: Ref,
        left: i64,
        down: bool,
        n: u32,
        w: u32,
        count: u32,
        lo: u32,
        site: u32,
    ) -> Value {
        let i = self.ld(idx, 0);
        let p = if down {
            let l = self.b.ins().iconst(I64, left);
            self.b.ins().isub(l, i)
        } else {
            self.b.ins().iadd_imm_s(i, -left)
        };
        // `p - (count - 1)` below `n - count + 1`, unsigned.
        let q = self.b.ins().iadd_imm_s(p, 1 - i64::from(count));
        let out = self.b.ins().icmp_imm_s(
            IntCC::UnsignedGreaterThanOrEqual,
            q,
            i64::from(n) - i64::from(count) + 1,
        );
        let p = if self.checks {
            self.check(out, site, &[i]);
            p
        } else {
            let z = self.konst(i64::from(count - 1) as u64);
            self.b.ins().select(out, z, p)
        };
        let l = self.b.ins().iconst(I64, i64::from(n - 1));
        let e = self.b.ins().isub(l, p);
        let e = if w == 1 {
            e
        } else {
            self.b.ins().imul_imm_s(e, i64::from(w))
        };
        self.b.ins().iadd_imm_s(e, i64::from(lo))
    }

    /// `x << (64 - s)` for `s` in `0 .. 64`: 0 when `s` is 0 (a shift by 64 is a shift by 0).
    fn shl_rest(&mut self, x: Value, s: Value) -> Value {
        let t = self.b.ins().ishl_imm_s(x, 1);
        let k = self.b.ins().iconst(I64, 63);
        let r = self.b.ins().isub(k, s);
        self.b.ins().ishl(t, r)
    }

    /// `x >> (64 - s)` for `s` in `0 .. 64`: 0 when `s` is 0.
    fn ushr_rest(&mut self, x: Value, s: Value) -> Value {
        let t = self.b.ins().ushr_imm_s(x, 1);
        let k = self.b.ins().iconst(I64, 63);
        let r = self.b.ins().isub(k, s);
        self.b.ins().ushr(t, r)
    }

    /// Whether a `w`-element field at a bit `lo + k * stride` never crosses a word.
    fn aligned(lo: u32, stride: u32, w: u32) -> bool {
        w.is_power_of_two() && w <= 64 && lo.is_multiple_of(w) && stride.is_multiple_of(w)
    }

    /// The `w` bits of `r` from the run-time bit `bit` (memory holds them, see `spill`).
    fn dyn_read(&mut self, r: Ref, bit: Value, w: u32, one_word: bool) -> Vec<Value> {
        (0..words(w))
            .map(|j| {
                let b = if j == 0 {
                    bit
                } else {
                    self.b.ins().iadd_imm_s(bit, i64::from(64 * j))
                };
                let (a, s) = self.bit_addr(r, b);
                let lo = self.b.ins().load(I64, MemFlagsData::trusted(), a, 0);
                let mut x = self.b.ins().ushr(lo, s);
                if !one_word {
                    // The next word may lie past the value: its bits are masked off below.
                    let hi = self.b.ins().load(I64, MemFlagsData::trusted(), a, 8);
                    let h = self.shl_rest(hi, s);
                    x = self.b.ins().bor(x, h);
                }
                let n = (w - 64 * j).min(64);
                if n < 64 {
                    x = self.b.ins().band_imm_s(x, mask(n) as i64);
                    self.zeros.insert(x, !mask(n));
                }
                x
            })
            .collect()
    }

    /// The `w` bits `v` into `r` from the run-time bit `bit`, in memory.
    fn dyn_write(&mut self, r: Ref, bit: Value, w: u32, v: &[Value], one_word: bool) {
        for (j, &x) in v.iter().enumerate() {
            let b = if j == 0 {
                bit
            } else {
                self.b.ins().iadd_imm_s(bit, 64 * j as i64)
            };
            let n = (w - 64 * j as u32).min(64);
            let (a, s) = self.bit_addr(r, b);
            let m = self.konst(mask(n));
            let w0 = self.b.ins().load(I64, MemFlagsData::trusted(), a, 0);
            let ms = self.b.ins().ishl(m, s);
            let keep = self.b.ins().band_not(w0, ms);
            let xs = self.b.ins().ishl(x, s);
            let w0 = self.b.ins().bor(keep, xs);
            self.b.ins().store(MemFlagsData::trusted(), w0, a, 0);
            if !one_word {
                // The bits that cross into the next word (none when `s` is 0); its other bits are
                // stored back unchanged.
                let w1 = self.b.ins().load(I64, MemFlagsData::trusted(), a, 8);
                let mh = self.ushr_rest(m, s);
                let keep = self.b.ins().band_not(w1, mh);
                let xh = self.ushr_rest(x, s);
                let w1 = self.b.ins().bor(keep, xh);
                self.b.ins().store(MemFlagsData::trusted(), w1, a, 8);
            }
        }
    }

    /// The value of `len` elements in `v` sign- or zero-extended to `to` elements.
    fn extend(&mut self, v: &[Value], len: u32, to: u32, signed: bool) -> Vec<Value> {
        let mut out: Vec<Value> = v[..words(len.min(to)) as usize].to_vec();
        if to <= len {
            self.top(&mut out, to);
            return out;
        }
        let k = ((len - 1) / 64) as usize;
        let fill = if signed {
            // The sign bit spread over a word: `(x << (63 - b)) >> 63`, arithmetic; the bits
            // above it in its word are zero (the invariant), so an OR sets them.
            let b = (len - 1) % 64;
            let t = self.b.ins().ishl_imm_s(v[k], i64::from(63 - b));
            let f = self.b.ins().sshr_imm_s(t, 63);
            if b != 63 {
                let hi = self.b.ins().band_imm_s(f, (!mask(b + 1)) as i64);
                out[k] = self.b.ins().bor(out[k], hi);
            }
            f
        } else {
            self.konst(0)
        };
        out.resize(words(to) as usize, fill);
        self.top(&mut out, to);
        out
    }

    /// `x < y` (`le`: `<=`) of two values of `n` words, unsigned, or signed by the top word.
    fn less(&mut self, x: &[Value], y: &[Value], signed: bool, le: bool) -> Value {
        // From the low word up: the result so far, overridden where a higher word differs.
        let mut r = if le {
            self.b.ins().iconst(cranelift_codegen::ir::types::I8, 1)
        } else {
            self.b.ins().iconst(cranelift_codegen::ir::types::I8, 0)
        };
        let nw = x.len();
        for k in 0..nw {
            let top = k + 1 == nw;
            let lt = if top && signed {
                IntCC::SignedLessThan
            } else {
                IntCC::UnsignedLessThan
            };
            let l = self.b.ins().icmp(lt, x[k], y[k]);
            let e = self.b.ins().icmp(IntCC::Equal, x[k], y[k]);
            r = self.b.ins().select(e, r, l);
        }
        r
    }

    /// The product of `x` and `y` modulo `2 ** (64 * nw)`, schoolbook on words.
    fn mul_words(&mut self, x: &[Value], y: &[Value], nw: usize) -> Vec<Value> {
        let zero = self.konst(0);
        let mut acc = vec![zero; nw];
        for (i, &a) in x.iter().enumerate().take(nw) {
            let mut carry = zero;
            for (j, &bb) in y.iter().enumerate() {
                let k = i + j;
                if k >= nw {
                    break;
                }
                let lo = self.b.ins().imul(a, bb);
                let hi = if k + 1 < nw {
                    self.b.ins().umulhi(a, bb)
                } else {
                    zero
                };
                // acc[k] + lo + carry, with the carries into the next word.
                let s1 = self.b.ins().iadd(acc[k], lo);
                let c1 = self.b.ins().icmp(IntCC::UnsignedLessThan, s1, lo);
                let s2 = self.b.ins().iadd(s1, carry);
                let c2 = self.b.ins().icmp(IntCC::UnsignedLessThan, s2, carry);
                acc[k] = s2;
                let c1 = self.bool_of(c1);
                let c2 = self.bool_of(c2);
                let c = self.b.ins().iadd(c1, c2);
                carry = self.b.ins().iadd(hi, c);
            }
            let k = i + y.len();
            if k < nw {
                acc[k] = self.b.ins().iadd(acc[k], carry);
            }
        }
        acc
    }

    /// The address of the word holding bit `bit` of `r`, and the bit's place in it.
    fn bit_addr(&mut self, r: Ref, bit: Value) -> (Value, Value) {
        let w = self.b.ins().ushr_imm_s(bit, 6);
        let w = self.b.ins().ishl_imm_s(w, 3);
        let w = if self.ptr == I64 {
            w
        } else {
            self.b.ins().ireduce(self.ptr, w)
        };
        let a = self.b.ins().iadd(self.base, w);
        let a = self.b.ins().iadd_imm_s(a, i64::from(word(r)));
        let s = self.b.ins().band_imm_s(bit, 63);
        (a, s)
    }

    fn op(&mut self, op: &Op) {
        match op {
            Op::Copy { dst, src, len } => {
                let v = self.lds(*src, *len);
                self.sts(*dst, &v);
            }
            Op::Lit { dst, val } => {
                let v: Vec<Value> = val.iter().map(|&w| self.konst(w)).collect();
                self.sts(*dst, &v);
            }
            Op::Blend { dst, src, mask } => {
                for (k, &m) in mask.iter().enumerate() {
                    if m == 0 {
                        continue;
                    }
                    let s = self.ld(*src, k as u32);
                    let x = if m == !0 {
                        s
                    } else {
                        let d = self.ld(*dst, k as u32);
                        let a = self.b.ins().band_imm_s(d, !m as i64);
                        let x = self.b.ins().band_imm_s(s, m as i64);
                        self.b.ins().bor(a, x)
                    };
                    self.st(*dst, k as u32, x);
                }
            }
            Op::Gate { g, dst, a, b, len } => {
                let (x, y) = (self.lds(*a, *len), self.lds(*b, *len));
                let mut v: Vec<Value> = x
                    .iter()
                    .zip(&y)
                    .map(|(&x, &y)| match g {
                        Gate::And => self.b.ins().band(x, y),
                        Gate::Or => self.b.ins().bor(x, y),
                        Gate::Xor => self.b.ins().bxor(x, y),
                        Gate::Nand => {
                            let t = self.b.ins().band(x, y);
                            self.b.ins().bnot(t)
                        }
                        Gate::Nor => {
                            let t = self.b.ins().bor(x, y);
                            self.b.ins().bnot(t)
                        }
                        Gate::Xnor => {
                            let t = self.b.ins().bxor(x, y);
                            self.b.ins().bnot(t)
                        }
                    })
                    .collect();
                if matches!(g, Gate::Nand | Gate::Nor | Gate::Xnor) {
                    self.top(&mut v, *len);
                }
                self.sts(*dst, &v);
            }
            Op::Not { dst, a, len } => {
                let x = self.lds(*a, *len);
                let mut v: Vec<Value> = x.iter().map(|&x| self.b.ins().bnot(x)).collect();
                self.top(&mut v, *len);
                self.sts(*dst, &v);
            }
            Op::Extract {
                dst,
                src,
                lo,
                len,
                full,
            } => {
                // Only the source words the bits are in, from word `k0`.
                let (k0, k1) = (lo / 64, (lo + len - 1) / 64);
                let s: Vec<Value> = (k0..=k1).map(|k| self.ld(*src, k)).collect();
                let base = 64 * k0;
                let v: Vec<Value> = (0..words(*len))
                    .map(|j| {
                        let off = lo + 64 * j - base;
                        let n = (*len - 64 * j).min(64);
                        self.bits(&s, off, n, full.saturating_sub(base))
                    })
                    .collect();
                self.sts(*dst, &v);
            }
            Op::Insert { dst, src, lo, len } => {
                let s = self.lds(*src, *len);
                // Only the words the elements go into, from word `k0`.
                let (k0, k1) = (lo / 64, (lo + len - 1) / 64);
                let mut d: Vec<Value> = (k0..=k1).map(|k| self.ld(*dst, k)).collect();
                self.insert(&mut d, &s, lo - 64 * k0, *len);
                for k in k0..=k1 {
                    self.st(*dst, k, d[(k - k0) as usize]);
                }
            }
            Op::Cat {
                dst,
                hi,
                lo,
                lo_len,
                len,
            } => {
                let l = self.lds(*lo, *lo_len);
                let h = self.lds(*hi, *len - *lo_len);
                let mut v = l.clone();
                v.resize(words(*len) as usize, l[0]);
                for x in v.iter_mut().skip(l.len()) {
                    *x = self.b.ins().iconst(I64, 0);
                }
                self.insert(&mut v, &h, *lo_len, *len - *lo_len);
                self.sts(*dst, &v);
            }
            Op::Add {
                dst,
                a,
                b,
                len,
                sub,
            } => {
                let (x, y) = (self.lds(*a, *len), self.lds(*b, *len));
                let mut v = Vec::new();
                let mut c: Option<Value> = None;
                let nw = x.len();
                for (k, (&x, &y)) in x.iter().zip(&y).enumerate() {
                    let last = k + 1 == nw;
                    let (r, c1) = if *sub {
                        let r = self.b.ins().isub(x, y);
                        (r, self.b.ins().icmp(IntCC::UnsignedLessThan, x, y))
                    } else {
                        let r = self.b.ins().iadd(x, y);
                        (r, self.b.ins().icmp(IntCC::UnsignedLessThan, r, x))
                    };
                    let (r, c1) = match c {
                        None => (r, c1),
                        Some(cin) => {
                            let ci = self.bool_of(cin);
                            if *sub {
                                let r2 = self.b.ins().isub(r, ci);
                                let c2 = self.b.ins().icmp(IntCC::UnsignedLessThan, r, ci);
                                (r2, self.b.ins().bor(c1, c2))
                            } else {
                                let r2 = self.b.ins().iadd(r, ci);
                                let c2 = self.b.ins().icmp(IntCC::UnsignedLessThan, r2, r);
                                (r2, self.b.ins().bor(c1, c2))
                            }
                        }
                    };
                    v.push(r);
                    c = (!last).then_some(c1);
                }
                self.top(&mut v, *len);
                self.sts(*dst, &v);
            }
            Op::VecEq { dst, a, b, len, ne } => {
                let (x, y) = (self.lds(*a, *len), self.lds(*b, *len));
                let mut d = self.konst(0);
                for (&x, &y) in x.iter().zip(&y) {
                    let t = self.b.ins().bxor(x, y);
                    d = self.b.ins().bor(d, t);
                }
                let cc = if *ne { IntCC::NotEqual } else { IntCC::Equal };
                let c = self.b.ins().icmp_imm_s(cc, d, 0);
                let r = self.bool_of(c);
                self.st(*dst, 0, r);
            }
            Op::IntLit { dst, v } => {
                let x = self.b.ins().iconst(I64, *v);
                self.st(*dst, 0, x);
            }
            Op::IntCmp { op, dst, a, b } => {
                let (x, y) = (self.ld(*a, 0), self.ld(*b, 0));
                let cc = match op {
                    Cmp::Eq => IntCC::Equal,
                    Cmp::Ne => IntCC::NotEqual,
                    Cmp::Lt => IntCC::SignedLessThan,
                    Cmp::Le => IntCC::SignedLessThanOrEqual,
                    Cmp::Gt => IntCC::SignedGreaterThan,
                    Cmp::Ge => IntCC::SignedGreaterThanOrEqual,
                };
                let c = self.b.ins().icmp(cc, x, y);
                let r = self.bool_of(c);
                self.st(*dst, 0, r);
            }
            Op::Int {
                op,
                dst,
                a,
                b,
                lo,
                hi,
                site,
            } => {
                let (x, y) = (self.ld(*a, 0), self.ld(*b, 0));
                let r = self.arith(*op, x, y, *lo, *hi, *site);
                self.st(*dst, 0, r);
            }
            Op::Range { a, lo, hi, site } => {
                if self.checks {
                    let x = self.ld(*a, 0);
                    let f = self.outside(x, *lo, *hi);
                    self.check(f, *site, &[x]);
                }
            }
            Op::IndexRead {
                dst,
                src,
                lo,
                n,
                stride,
                w,
                idx,
                left,
                down,
                site,
            } => {
                // Array elements the value spans: one for a part of one element.
                let count = (w / stride).max(1);
                let bit = self.element(*idx, *left, *down, *n, *stride, count, *lo, *site);
                let end = lo + n * stride;
                // The word after the value too: an element may reach into it.
                self.spill(*src, lo / 64, end.div_ceil(64));
                let v = self.dyn_read(*src, bit, *w, Self::aligned(*lo, *stride, *w));
                self.sts(*dst, &v);
            }
            Op::IndexWrite {
                dst,
                src,
                lo,
                n,
                stride,
                w,
                idx,
                left,
                down,
                site,
            } => {
                let v = self.lds(*src, *w);
                // Array elements the value spans: one for a part of one element.
                let count = (w / stride).max(1);
                let bit = self.element(*idx, *left, *down, *n, *stride, count, *lo, *site);
                let (k0, k1) = (lo / 64, (lo + n * stride).div_ceil(64));
                self.spill(*dst, k0, k1);
                self.forget(*dst, k0, k1);
                self.dyn_write(*dst, bit, *w, &v, Self::aligned(*lo, *stride, *w));
            }
            Op::Select { dst, c, a, b, len } => {
                let c = self.ld(*c, 0);
                let (x, y) = (self.lds(*a, *len), self.lds(*b, *len));
                let v: Vec<Value> = (x.iter().zip(&y))
                    .map(|(&x, &y)| self.b.ins().select(c, x, y))
                    .collect();
                self.sts(*dst, &v);
            }
            Op::Splat { dst, a, len } => {
                let x = self.ld(*a, 0);
                let z = self.konst(0);
                let ones = self.b.ins().isub(z, x);
                let mut v = vec![ones; words(*len) as usize];
                self.top(&mut v, *len);
                self.sts(*dst, &v);
            }
            Op::Ext {
                dst,
                src,
                from,
                to,
                signed,
            } => {
                let x = self.lds(*src, *from);
                let v = self.extend(&x, *from, *to, *signed);
                self.sts(*dst, &v);
            }
            Op::VCmp {
                cmp,
                signed,
                dst,
                a,
                b,
                len,
            } => {
                let full = 64 * words(*len);
                let x = self.lds(*a, *len);
                let y = self.lds(*b, *len);
                let (x, y) = if *signed {
                    (
                        self.extend(&x, *len, full, true),
                        self.extend(&y, *len, full, true),
                    )
                } else {
                    (x, y)
                };
                let c = match cmp {
                    Cmp::Lt => self.less(&x, &y, *signed, false),
                    Cmp::Le => self.less(&x, &y, *signed, true),
                    Cmp::Gt => self.less(&y, &x, *signed, false),
                    Cmp::Ge => self.less(&y, &x, *signed, true),
                    Cmp::Eq | Cmp::Ne => {
                        let mut d = self.konst(0);
                        for (&x, &y) in x.iter().zip(&y) {
                            let t = self.b.ins().bxor(x, y);
                            d = self.b.ins().bor(d, t);
                        }
                        let cc = if *cmp == Cmp::Ne {
                            IntCC::NotEqual
                        } else {
                            IntCC::Equal
                        };
                        self.b.ins().icmp_imm_s(cc, d, 0)
                    }
                };
                let r = self.bool_of(c);
                self.st(*dst, 0, r);
            }
            Op::Mul { dst, a, b, len } => {
                let (x, y) = (self.lds(*a, *len), self.lds(*b, *len));
                let mut v = self.mul_words(&x, &y, words(*len) as usize);
                self.top(&mut v, *len);
                self.sts(*dst, &v);
            }
            Op::VDiv {
                op,
                signed,
                dst,
                a,
                b,
                len,
                site,
            } => {
                // One word (`len` at most 64): extend to the word, divide, cut back.
                let x = self.ld(*a, 0);
                let y = self.ld(*b, 0);
                let (x, y) = if *signed {
                    let e = |g: &mut Self, v| g.extend(&[v], *len, 64, true)[0];
                    (e(self, x), e(self, y))
                } else {
                    (x, y)
                };
                let z = self.b.ins().icmp_imm_s(IntCC::Equal, y, 0);
                // `MIN / -1` traps in the machine: only a 64-element signed operand can be it.
                let f = if *signed && *len == 64 {
                    let m1 = self.b.ins().icmp_imm_s(IntCC::Equal, y, -1);
                    let mn = self.b.ins().icmp_imm_s(IntCC::Equal, x, i64::MIN);
                    let ov = self.b.ins().band(m1, mn);
                    self.b.ins().bor(z, ov)
                } else {
                    z
                };
                self.check(f, *site, &[x, y]);
                let r = match (op, signed) {
                    (Arith::Div, false) => self.b.ins().udiv(x, y),
                    (Arith::Div, true) => self.b.ins().sdiv(x, y),
                    (_, false) => self.b.ins().urem(x, y),
                    (_, true) => {
                        let r = self.b.ins().srem(x, y);
                        if *op == Arith::Mod {
                            self.mod_fix(r, y)
                        } else {
                            r
                        }
                    }
                };
                let mut v = vec![r];
                self.top(&mut v, *len);
                self.st(*dst, 0, v[0]);
            }
            Op::Shift { k, dst, a, len, n } => {
                // One word: the count from the integer word `n`, at least 0.
                let x = self.ld(*a, 0);
                let c = self.ld(*n, 0);
                let l = i64::from(*len);
                let r = match k {
                    ShiftK::Sll | ShiftK::Srl | ShiftK::Sra => {
                        let big = self
                            .b
                            .ins()
                            .icmp_imm_s(IntCC::UnsignedGreaterThanOrEqual, c, l);
                        let x2 = if *k == ShiftK::Sra {
                            self.extend(&[x], *len, 64, true)[0]
                        } else {
                            x
                        };
                        let cm = self.b.ins().band_imm_s(c, 63);
                        let s = match k {
                            ShiftK::Sll => self.b.ins().ishl(x2, cm),
                            ShiftK::Srl => self.b.ins().ushr(x2, cm),
                            _ => self.b.ins().sshr(x2, cm),
                        };
                        let full = if *k == ShiftK::Sra {
                            self.b.ins().sshr_imm_s(x2, 63)
                        } else {
                            self.konst(0)
                        };
                        self.b.ins().select(big, full, s)
                    }
                    ShiftK::Rol | ShiftK::Ror => {
                        // By `m = c mod len`: `x << p | x >> q` with `p + q = len`, both below 64
                        // unless `m` is 0, which is `x` itself.
                        let m = self.b.ins().urem_imm_s(c, l);
                        let lk = self.b.ins().iconst(I64, l);
                        let back = self.b.ins().isub(lk, m);
                        let (p, q) = if *k == ShiftK::Rol {
                            (m, back)
                        } else {
                            (back, m)
                        };
                        let hi = self.b.ins().ishl(x, p);
                        let lo = self.b.ins().ushr(x, q);
                        let r = self.b.ins().bor(hi, lo);
                        let none = self.b.ins().icmp_imm_s(IntCC::Equal, m, 0);
                        self.b.ins().select(none, x, r)
                    }
                };
                let mut v = vec![r];
                self.top(&mut v, *len);
                self.st(*dst, 0, v[0]);
            }
            Op::Reduce { g, dst, a, len } => {
                let x = self.lds(*a, *len);
                let r = match g {
                    Gate::And | Gate::Nand => {
                        let mut d = self.konst(0);
                        for (k, &w) in x.iter().enumerate() {
                            let m = mask((*len - 64 * k as u32).min(64));
                            let t = self.b.ins().bxor_imm_s(w, m as i64);
                            d = self.b.ins().bor(d, t);
                        }
                        self.b.ins().icmp_imm_s(IntCC::Equal, d, 0)
                    }
                    Gate::Or | Gate::Nor => {
                        let mut d = self.konst(0);
                        for &w in &x {
                            d = self.b.ins().bor(d, w);
                        }
                        self.b.ins().icmp_imm_s(IntCC::NotEqual, d, 0)
                    }
                    Gate::Xor | Gate::Xnor => {
                        let mut d = self.konst(0);
                        for &w in &x {
                            d = self.b.ins().bxor(d, w);
                        }
                        let p = self.b.ins().popcnt(d);
                        let p = self.b.ins().band_imm_s(p, 1);
                        self.b.ins().icmp_imm_s(IntCC::NotEqual, p, 0)
                    }
                };
                let mut r = self.bool_of(r);
                if matches!(g, Gate::Nand | Gate::Nor | Gate::Xnor) {
                    r = self.b.ins().bxor_imm_s(r, 1);
                }
                self.st(*dst, 0, r);
            }
            Op::ToInt {
                dst,
                a,
                len,
                signed,
            } => {
                let x = self.ld(*a, 0);
                let r = if *signed {
                    self.extend(&[x], *len, 64, true)[0]
                } else {
                    x
                };
                self.st(*dst, 0, r);
            }
            Op::FromInt { dst, a, len } => {
                let x = self.ld(*a, 0);
                let s = self.b.ins().sshr_imm_s(x, 63);
                let mut v = vec![s; words(*len) as usize];
                v[0] = x;
                self.top(&mut v, *len);
                self.sts(*dst, &v);
            }
            Op::If { cond, then, else_ } => {
                let c = self.ld(*cond, 0);
                let (t, e, j): (CBlock, CBlock, CBlock) = (
                    self.b.create_block(),
                    self.b.create_block(),
                    self.b.create_block(),
                );
                // Memory holds every word before the branches; each starts from the words in
                // registers, stores what it changed before the join, and the join reads those
                // from memory again.
                let dirty: Vec<(i32, Value)> = (self.cache.iter_mut())
                    .filter(|(_, (_, d))| *d)
                    .map(|(&off, (v, d))| {
                        *d = false;
                        (off, *v)
                    })
                    .collect();
                for (off, v) in dirty {
                    self.store(off, v);
                }
                self.b.ins().brif(c, t, &[], e, &[]);
                let before = self.cache.clone();
                let mut touched = Vec::new();
                for (blk, body) in [(t, then), (e, else_)] {
                    self.b.switch_to_block(blk);
                    self.b.seal_block(blk);
                    self.cache = before.clone();
                    self.ops(body);
                    let made: Vec<(i32, Value, bool)> = (self.cache.iter())
                        .filter(|(off, x)| before.get(off) != Some(x))
                        .map(|(&off, &(v, d))| (off, v, d))
                        .collect();
                    for (off, v, d) in made {
                        if d {
                            self.store(off, v);
                        }
                        touched.push(off);
                    }
                    // Words the branch dropped (it wrote them in memory) are stale after it too.
                    touched.extend(before.keys().filter(|k| !self.cache.contains_key(k)));
                    self.b.ins().jump(j, &[]);
                }
                self.b.switch_to_block(j);
                self.b.seal_block(j);
                self.cache = before;
                for off in touched {
                    self.cache.remove(&off);
                }
            }
        }
    }

    /// `mod` from the remainder `r` of a division by `y`: the sign of the divisor (LRM 9.2.7),
    /// `r + y` when `r` and `y` differ in sign.
    fn mod_fix(&mut self, r: Value, y: Value) -> Value {
        let s = self.b.ins().bxor(r, y);
        let neg = self.b.ins().icmp_imm_s(IntCC::SignedLessThan, s, 0);
        let nz = self.b.ins().icmp_imm_s(IntCC::NotEqual, r, 0);
        let fix = self.b.ins().band(neg, nz);
        let ry = self.b.ins().iadd(r, y);
        self.b.ins().select(fix, ry, r)
    }

    /// Whether `x` is outside `lo ..= hi`.
    fn outside(&mut self, x: Value, lo: i64, hi: i64) -> Value {
        let a = self.b.ins().icmp_imm_s(IntCC::SignedLessThan, x, lo);
        let b = self.b.ins().icmp_imm_s(IntCC::SignedGreaterThan, x, hi);
        self.b.ins().bor(a, b)
    }

    /// `x op y` in `lo ..= hi`, the result type: a failed check stores both operands.
    fn arith(&mut self, op: Arith, x: Value, y: Value, lo: i64, hi: i64, site: u32) -> Value {
        match op {
            Arith::Add | Arith::Sub | Arith::Mul => {
                if !self.checks {
                    return match op {
                        Arith::Add => self.b.ins().iadd(x, y),
                        Arith::Sub => self.b.ins().isub(x, y),
                        _ => self.b.ins().imul(x, y),
                    };
                }
                let (r, of) = match op {
                    Arith::Add => self.b.ins().sadd_overflow(x, y),
                    Arith::Sub => self.b.ins().ssub_overflow(x, y),
                    _ => self.b.ins().smul_overflow(x, y),
                };
                let out = self.outside(r, lo, hi);
                let f = self.b.ins().bor(of, out);
                self.check(f, site, &[x, y]);
                r
            }
            Arith::Div | Arith::Mod | Arith::Rem => {
                // Always guarded: the machine division traps on zero and on MIN / -1.
                let z = self.b.ins().icmp_imm_s(IntCC::Equal, y, 0);
                let m1 = self.b.ins().icmp_imm_s(IntCC::Equal, y, -1);
                let mn = self.b.ins().icmp_imm_s(IntCC::Equal, x, i64::MIN);
                let ov = self.b.ins().band(m1, mn);
                let f = self.b.ins().bor(z, ov);
                self.check(f, site, &[x, y]);
                let r = match op {
                    Arith::Div => self.b.ins().sdiv(x, y),
                    _ => self.b.ins().srem(x, y),
                };
                let r = if op == Arith::Mod {
                    self.mod_fix(r, y)
                } else {
                    r
                };
                if self.checks && op == Arith::Div {
                    let out = self.outside(r, lo, hi);
                    self.check(out, site, &[x, y]);
                }
                r
            }
        }
    }
}

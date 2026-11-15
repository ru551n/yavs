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

use cranelift_codegen::ir::condcodes::IntCC;
use cranelift_codegen::ir::types::{I32, I64};
use cranelift_codegen::ir::{AbiParam, Block as CBlock, InstBuilder, MemFlagsData, Value};
use cranelift_codegen::settings::{self, Configurable};
use cranelift_frontend::{FunctionBuilder, FunctionBuilderContext};
use cranelift_jit::{JITBuilder, JITModule};
use cranelift_module::{FuncId, Linkage, Module};

use crate::lower::{Arith, Cmp, Gate, Op, Ref};

/// One compiled trigger function: the address of the image's first word; 0, or the failed check's
/// site plus one.
pub(crate) type Entry = unsafe extern "C" fn(*mut u64) -> u32;

/// The compiled functions of a region; the module owns their code.
pub(crate) struct Code {
    _module: JITModule,
    settle: Entry,
    edge: Entry,
    /// The words the code addresses.
    words: usize,
}

impl Code {
    /// Run the settle function (`edge` false) or the edge function on `img`: `Err(site)` when a
    /// check failed.
    pub(crate) fn run(&self, img: &mut [u64], edge: bool) -> Result<(), u32> {
        assert!(
            img.len() >= self.words,
            "the image is smaller than the code's words"
        );
        let f = if edge { self.edge } else { self.settle };
        // SAFETY: the code reads and writes only words below `words`, checked above.
        match unsafe { f(img.as_mut_ptr()) } {
            0 => Ok(()),
            s => Err(s - 1),
        }
    }
}

// SAFETY: the code is finished and never changed again; the module is only kept alive.
unsafe impl Send for Code {}
// SAFETY: as above, nothing in it is mutated after `compile`.
unsafe impl Sync for Code {}

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
        Ref::Sig(_) => {
            unreachable!("signal references are resolved to slots before code generation")
        }
    }
}

/// Compile the two trigger functions. Every `Ref` of `settle` and `edge` is an image slot
/// (`Ref::Loc`); `err` is the first of the two error words. `name` labels the functions in the
/// perf map.
pub(crate) fn compile(
    settle: &[Op],
    edge: &[Op],
    words: usize,
    err: u32,
    name: &str,
) -> Result<Code, String> {
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
    for (what, ops) in [("settle", settle), ("edge", edge)] {
        let sym = format!("cycle_{what}_{name}");
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
            };
            g.ops(ops);
            let zero = g.b.ins().iconst(I32, 0);
            g.b.ins().return_(&[zero]);
            g.b.finalize(module.target_config());
        }
        module
            .define_function(id, &mut ctx)
            .map_err(|e| format!("{e:?}"))?;
        ids.push((id, sym));
    }
    module.finalize_definitions().map_err(|e| e.to_string())?;
    let addr = |id: FuncId| module.get_finalized_function(id);
    let (settle, edge) = (addr(ids[0].0), addr(ids[1].0));
    if perf {
        for (id, sym) in &ids {
            // The size is not kept by the module; one page is an upper bound for a label.
            yavs_rt::perfmap::append(addr(*id) as usize, 4096, sym);
        }
    }
    // SAFETY: both are functions of signature `fn(ptr) -> u32` defined above.
    let (settle, edge) = unsafe {
        (
            std::mem::transmute::<*const u8, Entry>(settle),
            std::mem::transmute::<*const u8, Entry>(edge),
        )
    };
    Ok(Code {
        _module: module,
        settle,
        edge,
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
}

impl Gen<'_> {
    fn ops(&mut self, ops: &[Op]) {
        for op in ops {
            self.op(op);
        }
    }

    fn load_at(&mut self, off: i32) -> Value {
        self.b
            .ins()
            .load(I64, MemFlagsData::trusted(), self.base, off)
    }

    /// Word `k` of `r`.
    fn ld(&mut self, r: Ref, k: u32) -> Value {
        self.load_at(word(r) + k as i32 * 8)
    }

    fn st(&mut self, r: Ref, k: u32, v: Value) {
        self.b.ins().store(
            MemFlagsData::trusted(),
            v,
            self.base,
            word(r) + k as i32 * 8,
        );
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
        self.b.ins().iconst(I64, v as i64)
    }

    /// The top word of a value of `len` elements with the bits above `len` cleared.
    fn top(&mut self, v: &mut [Value], len: u32) {
        if !len.is_multiple_of(64)
            && let Some(t) = v.last_mut()
        {
            *t = self.b.ins().band_imm_s(*t, mask(len % 64) as i64);
        }
    }

    /// `n` (at most 64) bits of the words `v` from bit `off`, in the low bits.
    fn bits(&mut self, v: &[Value], off: u32, n: u32) -> Value {
        let (k, s) = ((off / 64) as usize, off % 64);
        let mut x = v[k];
        if s != 0 {
            x = self.b.ins().ushr_imm_s(x, i64::from(s));
            if s + n > 64 {
                let h = self.b.ins().ishl_imm_s(v[k + 1], i64::from(64 - s));
                x = self.b.ins().bor(x, h);
            }
        }
        if n < 64 {
            x = self.b.ins().band_imm_s(x, mask(n) as i64);
        }
        x
    }

    /// `d` with bits `lo .. lo + len` taken from `s` (`len` elements).
    fn insert(&mut self, d: &mut [Value], s: &[Value], lo: u32, len: u32) {
        let mut a = lo;
        while a < lo + len {
            let k = a / 64;
            let b = (lo + len).min(k * 64 + 64);
            let x = self.bits(s, a - lo, b - a);
            let sh = a - k * 64;
            let m = mask(b - a) << sh;
            let keep = self.b.ins().band_imm_s(d[k as usize], !m as i64);
            let x = if sh != 0 {
                self.b.ins().ishl_imm_s(x, i64::from(sh))
            } else {
                x
            };
            d[k as usize] = self.b.ins().bor(keep, x);
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
    /// below `n` (or clamped to 0 when checks are off); returns its bit, `lo + n - 1 - p`.
    fn element(&mut self, idx: Ref, left: i64, down: bool, n: u32, lo: u32, site: u32) -> Value {
        let i = self.ld(idx, 0);
        let p = if down {
            let l = self.b.ins().iconst(I64, left);
            self.b.ins().isub(l, i)
        } else {
            self.b.ins().iadd_imm_s(i, -left)
        };
        let out = self
            .b
            .ins()
            .icmp_imm_s(IntCC::UnsignedGreaterThanOrEqual, p, i64::from(n));
        let p = if self.checks {
            self.check(out, site, &[i]);
            p
        } else {
            let z = self.konst(0);
            self.b.ins().select(out, z, p)
        };
        let l = self.b.ins().iconst(I64, i64::from(lo + n - 1));
        self.b.ins().isub(l, p)
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
            Op::Extract { dst, src, lo, len } => {
                // Only the source words the bits are in.
                let (k0, k1) = (lo / 64, (lo + len - 1) / 64);
                let mut s: Vec<Value> = Vec::new();
                for k in 0..=k1 {
                    s.push(if k < k0 {
                        self.konst(0)
                    } else {
                        self.ld(*src, k)
                    });
                }
                let v: Vec<Value> = (0..words(*len))
                    .map(|j| {
                        let off = lo + 64 * j;
                        let n = (*len - 64 * j).min(64);
                        self.bits(&s, off, n)
                    })
                    .collect();
                self.sts(*dst, &v);
            }
            Op::Insert { dst, src, lo, len } => {
                let s = self.lds(*src, *len);
                let (k0, k1) = (lo / 64, (lo + len - 1) / 64);
                let mut d: Vec<Value> = Vec::new();
                for k in 0..=k1 {
                    d.push(if k < k0 {
                        self.konst(0)
                    } else {
                        self.ld(*dst, k)
                    });
                }
                self.insert(&mut d, &s, *lo, *len);
                for k in k0..=k1 {
                    self.st(*dst, k, d[k as usize]);
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
                idx,
                left,
                down,
                site,
            } => {
                let bit = self.element(*idx, *left, *down, *n, *lo, *site);
                let (a, s) = self.bit_addr(*src, bit);
                let w = self.b.ins().load(I64, MemFlagsData::trusted(), a, 0);
                let x = self.b.ins().ushr(w, s);
                let x = self.b.ins().band_imm_s(x, 1);
                self.st(*dst, 0, x);
            }
            Op::IndexWrite {
                dst,
                src,
                lo,
                n,
                idx,
                left,
                down,
                site,
            } => {
                let v = self.ld(*src, 0);
                let bit = self.element(*idx, *left, *down, *n, *lo, *site);
                let (a, s) = self.bit_addr(*dst, bit);
                let w = self.b.ins().load(I64, MemFlagsData::trusted(), a, 0);
                let one = self.konst(1);
                let m = self.b.ins().ishl(one, s);
                let m = self.b.ins().bnot(m);
                let w = self.b.ins().band(w, m);
                let x = self.b.ins().ishl(v, s);
                let w = self.b.ins().bor(w, x);
                self.b.ins().store(MemFlagsData::trusted(), w, a, 0);
            }
            Op::If { cond, then, else_ } => {
                let c = self.ld(*cond, 0);
                let (t, e, j): (CBlock, CBlock, CBlock) = (
                    self.b.create_block(),
                    self.b.create_block(),
                    self.b.create_block(),
                );
                self.b.ins().brif(c, t, &[], e, &[]);
                for (blk, body) in [(t, then), (e, else_)] {
                    self.b.switch_to_block(blk);
                    self.b.seal_block(blk);
                    self.ops(body);
                    self.b.ins().jump(j, &[]);
                }
                self.b.switch_to_block(j);
                self.b.seal_block(j);
            }
        }
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
                    // The sign of the divisor (LRM 9.2.7): `r + y` when `r` and `y` differ in sign.
                    let s = self.b.ins().bxor(r, y);
                    let neg = self.b.ins().icmp_imm_s(IntCC::SignedLessThan, s, 0);
                    let nz = self.b.ins().icmp_imm_s(IntCC::NotEqual, r, 0);
                    let fix = self.b.ins().band(neg, nz);
                    let ry = self.b.ins().iadd(r, y);
                    self.b.ins().select(fix, ry, r)
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

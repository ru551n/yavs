//! Cranelift code of the region (design 6.4): one function per trigger, the operations of the
//! member bodies in schedule order on the flat state image. Milestone 1's values are four-plane
//! blocks; the IEEE 1164 gates are calls into `yavs-logic` (design 5.8), the rest is inline.
//!
//! The only `unsafe` of the crate: the helpers read and write the image through raw pointers,
//! and the region calls the generated code.
#![allow(unsafe_code)]

use cranelift_codegen::ir::types::{I32, I64};
use cranelift_codegen::ir::{AbiParam, InstBuilder, MemFlagsData, Value};
use cranelift_codegen::settings::{self, Configurable};
use cranelift_frontend::{FunctionBuilder, FunctionBuilderContext};
use cranelift_jit::{JITBuilder, JITModule};
use cranelift_module::{FuncId, Linkage, Module};
use yavs_logic::BinOp;
use yavs_logic::{Block, Planes};

use crate::lower::{Gate, Op, Ref};

/// One compiled trigger function: the address of the image's first block.
pub(crate) type Entry = unsafe extern "C" fn(*mut Block);

/// The compiled functions of a region; the module owns their code.
pub(crate) struct Code {
    _module: JITModule,
    settle: Entry,
    edge: Entry,
    /// The blocks the code addresses.
    slots: usize,
}

impl Code {
    /// Run the settle function (`edge` false) or the edge function on `img`.
    pub(crate) fn run(&self, img: &mut [Block], edge: bool) {
        assert!(
            img.len() >= self.slots,
            "the image is smaller than the code's slots"
        );
        let f = if edge { self.edge } else { self.settle };
        // SAFETY: the code reads and writes only blocks below `slots`, checked above.
        unsafe { f(img.as_mut_ptr()) }
    }
}

// SAFETY: the code is finished and never changed again; the module is only kept alive.
unsafe impl Send for Code {}
// SAFETY: as above, nothing in it is mutated after `compile`.
unsafe impl Sync for Code {}

fn mask(len: u32) -> u64 {
    if len >= 64 { !0 } else { (1u64 << len) - 1 }
}

/// IEEE 1164 gate `g` of the blocks at `a` and `b` into `dst` (`len` elements).
///
/// # Safety
/// The three pointers address blocks of the image.
unsafe extern "C" fn yavs_cycle_gate(
    g: u32,
    a: *const Block,
    b: *const Block,
    dst: *mut Block,
    len: u32,
) {
    // SAFETY: the generated code passes addresses of blocks of the image (the caller's contract).
    let (x, y) = unsafe { (*a, *b) };
    let (op, inv) = match g {
        0 => (BinOp::And, false),
        1 => (BinOp::Or, false),
        2 => (BinOp::Xor, false),
        3 => (BinOp::And, true),
        4 => (BinOp::Or, true),
        _ => (BinOp::Xor, true),
    };
    let mut r = Planes::bin_block(op, x, y);
    if inv {
        r = Planes::not_block(r);
    }
    let m = mask(len);
    // SAFETY: as above.
    unsafe { *dst = r.map(|p| p & m) };
}

/// IEEE 1164 `not` of the block at `a` into `dst`.
///
/// # Safety
/// Both pointers address blocks of the image.
unsafe extern "C" fn yavs_cycle_not(a: *const Block, dst: *mut Block, len: u32) {
    // SAFETY: the generated code passes addresses of blocks of the image.
    let r = Planes::not_block(unsafe { *a });
    let m = mask(len);
    // SAFETY: as above.
    unsafe { *dst = r.map(|p| p & m) };
}

fn slot(r: Ref) -> i32 {
    match r {
        Ref::Loc(s) => (s * 32) as i32,
        Ref::Sig(_) => {
            unreachable!("signal references are resolved to slots before code generation")
        }
    }
}

/// Compile the two trigger functions. Every `Ref` of `settle` and `edge` is an image slot
/// (`Ref::Loc`). `name` labels the functions in the perf map.
pub(crate) fn compile(
    settle: &[Op],
    edge: &[Op],
    slots: usize,
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
    let mut jb = JITBuilder::with_isa(isa, cranelift_module::default_libcall_names());
    jb.symbol("yavs_cycle_gate", yavs_cycle_gate as *const u8);
    jb.symbol("yavs_cycle_not", yavs_cycle_not as *const u8);
    let mut module = JITModule::new(jb);
    let ptr = module.target_config().pointer_type();
    let mut sig = module.make_signature();
    sig.params
        .extend([I32, ptr, ptr, ptr, I32].map(AbiParam::new));
    let gate = module
        .declare_function("yavs_cycle_gate", Linkage::Import, &sig)
        .map_err(|e| e.to_string())?;
    let mut sig = module.make_signature();
    sig.params.extend([ptr, ptr, I32].map(AbiParam::new));
    let not = module
        .declare_function("yavs_cycle_not", Linkage::Import, &sig)
        .map_err(|e| e.to_string())?;
    let mut ids = Vec::new();
    let perf = std::env::var_os("YAVS_CLIFT_PERFMAP").is_some();
    for (what, ops) in [("settle", settle), ("edge", edge)] {
        let sym = format!("cycle_{what}_{name}");
        let mut sig = module.make_signature();
        sig.params.push(AbiParam::new(ptr));
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
            let gate = module.declare_func_in_func(gate, b.func);
            let not = module.declare_func_in_func(not, b.func);
            let mut g = Gen { b, base };
            for op in ops {
                g.op(op, gate, not);
            }
            g.b.ins().return_(&[]);
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
    // SAFETY: both are functions of signature `fn(ptr)` defined above.
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
        slots,
    })
}

struct Gen<'a> {
    b: FunctionBuilder<'a>,
    base: Value,
}

impl Gen<'_> {
    fn load(&mut self, r: Ref) -> [Value; 4] {
        let off = slot(r);
        [0, 1, 2, 3].map(|k| {
            self.b
                .ins()
                .load(I64, MemFlagsData::trusted(), self.base, off + k * 8)
        })
    }

    fn store(&mut self, r: Ref, v: [Value; 4]) {
        let off = slot(r);
        for (k, x) in v.into_iter().enumerate() {
            self.b
                .ins()
                .store(MemFlagsData::trusted(), x, self.base, off + k as i32 * 8);
        }
    }

    fn addr(&mut self, r: Ref) -> Value {
        self.b.ins().iadd_imm_s(self.base, i64::from(slot(r)))
    }

    fn op(
        &mut self,
        op: &Op,
        gate: cranelift_codegen::ir::FuncRef,
        not: cranelift_codegen::ir::FuncRef,
    ) {
        match *op {
            Op::Copy { dst, src } => {
                let v = self.load(src);
                self.store(dst, v);
            }
            Op::Lit { dst, val } => {
                let v = val.map(|w| self.b.ins().iconst(I64, w as i64));
                self.store(dst, v);
            }
            Op::Extract { dst, src, lo, len } => {
                let v = self.load(src);
                let m = mask(len) as i64;
                let v = v.map(|w| {
                    let s = self.b.ins().ushr_imm_s(w, i64::from(lo));
                    self.b.ins().band_imm_s(s, m)
                });
                self.store(dst, v);
            }
            Op::Insert { dst, src, lo, len } => {
                let (d, s) = (self.load(dst), self.load(src));
                let m = mask(len);
                let keep = !(m << lo) as i64;
                let mut out = d;
                for k in 0..4 {
                    let a = self.b.ins().band_imm_s(d[k], keep);
                    let x = self.b.ins().band_imm_s(s[k], m as i64);
                    let x = self.b.ins().ishl_imm_s(x, i64::from(lo));
                    out[k] = self.b.ins().bor(a, x);
                }
                self.store(dst, out);
            }
            Op::Cat {
                dst,
                hi,
                lo,
                lo_len,
            } => {
                let (h, l) = (self.load(hi), self.load(lo));
                let mut out = l;
                if lo_len < 64 {
                    for k in 0..4 {
                        let x = self.b.ins().ishl_imm_s(h[k], i64::from(lo_len));
                        out[k] = self.b.ins().bor(x, l[k]);
                    }
                }
                self.store(dst, out);
            }
            Op::AddK { dst, a, len, k } => {
                // A known element has code 2, 3, 6 or 7: plane 1 set, plane 3 clear.
                let v = self.load(a);
                let m = mask(len) as i64;
                let n3 = self.b.ins().bnot(v[3]);
                let known = self.b.ins().band(v[1], n3);
                let known = self.b.ins().band_imm_s(known, m);
                let ok = self.b.ins().icmp_imm_s(
                    cranelift_codegen::ir::condcodes::IntCC::Equal,
                    known,
                    m,
                );
                let sum = self.b.ins().iadd_imm_s(v[0], k as i64);
                let sum = self.b.ins().band_imm_s(sum, m);
                let mv = self.b.ins().iconst(I64, m);
                let zero = self.b.ins().iconst(I64, 0);
                // All `X` (code 1: plane 0) otherwise.
                let p0 = self.b.ins().select(ok, sum, mv);
                let p1 = self.b.ins().select(ok, mv, zero);
                self.store(dst, [p0, p1, zero, zero]);
            }
            Op::Gate { g, dst, a, b, len } => {
                let code = match g {
                    Gate::And => 0,
                    Gate::Or => 1,
                    Gate::Xor => 2,
                    Gate::Nand => 3,
                    Gate::Nor => 4,
                    Gate::Xnor => 5,
                };
                let args = [
                    self.b.ins().iconst(I32, code),
                    self.addr(a),
                    self.addr(b),
                    self.addr(dst),
                    self.b.ins().iconst(I32, i64::from(len)),
                ];
                self.b.ins().call(gate, &args);
            }
            Op::Not { dst, a, len } => {
                let args = [
                    self.addr(a),
                    self.addr(dst),
                    self.b.ins().iconst(I32, i64::from(len)),
                ];
                self.b.ins().call(not, &args);
            }
        }
    }
}

//! Safe-ish wrappers around ORC LLJIT, LLVM modules and the new pass manager.

use std::ffi::{CStr, CString, c_char, c_void};
use std::fmt;
use std::ptr;
use std::sync::{Mutex, Once};

use llvm_sys::core::*;
use llvm_sys::error::*;
use llvm_sys::orc2::lljit::*;
use llvm_sys::orc2::*;
use llvm_sys::prelude::*;

use crate::OptLevel;
use llvm_sys::target::*;
use llvm_sys::target_machine::*;
use llvm_sys::transforms::pass_builder::*;

/// An error reported by LLVM.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Error(pub String);

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "llvm: {}", self.0)
    }
}

impl std::error::Error for Error {}

type Result<T> = std::result::Result<T, Error>;

/// Consumes an `LLVMErrorRef`.
fn check(err: LLVMErrorRef) -> Result<()> {
    if err.is_null() {
        return Ok(());
    }
    // SAFETY: err is a live error; LLVMGetErrorMessage consumes it.
    unsafe {
        let msg = LLVMGetErrorMessage(err);
        let s = CStr::from_ptr(msg).to_string_lossy().into_owned();
        LLVMDisposeErrorMessage(msg);
        Err(Error(s))
    }
}

/// Takes ownership of a message allocated by LLVM.
unsafe fn take_message(msg: *mut c_char) -> String {
    if msg.is_null() {
        return String::new();
    }
    // SAFETY: caller passes a NUL-terminated LLVM message.
    unsafe {
        let s = CStr::from_ptr(msg).to_string_lossy().into_owned();
        LLVMDisposeMessage(msg);
        s
    }
}

/// Code generator switches (LLVM command line options, process-wide, set once before the first
/// target machine). Instruction scheduling and the machine-level motion and clean-up passes
/// (sinking, LICM, CSE, copy propagation, post-RA scheduling, if-conversion, tail duplication)
/// cost about a quarter of the codegen time of the generated code and change the speed of the
/// compiled code by less than the run-to-run noise of the benchmarks (#432); instruction
/// selection, register allocation, peepholes, branch folding and block placement stay (the
/// placement keeps the hot path straight: fewer taken branches, 20 percent fewer branch misses
/// and 5 to 9 percent fewer cycles on compiled clocked RTL, #634).
const CODEGEN_ARGS: &str = "-enable-misched=false -enable-post-misched=false \
    -disable-machine-sink -disable-machine-licm -disable-machine-cse \
    -disable-copyprop -disable-post-ra -disable-early-ifcvt -disable-postra-machine-licm \
    -disable-postra-machine-sink -disable-early-taildup";

/// Copies and disposes an LLVM memory buffer.
unsafe fn take_buffer(buf: LLVMMemoryBufferRef) -> Vec<u8> {
    if buf.is_null() {
        return Vec::new();
    }
    // SAFETY: the caller passes a live buffer, which is ours to dispose.
    unsafe {
        let v = std::slice::from_raw_parts(
            LLVMGetBufferStart(buf).cast::<u8>(),
            LLVMGetBufferSize(buf),
        )
        .to_vec();
        LLVMDisposeMemoryBuffer(buf);
        v
    }
}

/// What machine code depends on besides the module, the level and `optimize_as`'s `full`: the
/// target triple, the CPU baseline, the LLVM version, the code generator switches and a custom
/// pipeline (part of the key of the object cache).
pub fn target_id() -> String {
    init();
    let (mut major, mut minor, mut patch) = (0, 0, 0);
    // SAFETY: plain queries; the triple is an owned LLVM message.
    unsafe {
        LLVMGetVersion(&mut major, &mut minor, &mut patch);
        let triple = take_message(LLVMGetDefaultTargetTriple());
        format!(
            "{triple} {} llvm-{major}.{minor}.{patch} model={:?}/{:?} args={} pipeline={}",
            portable_cpu().to_string_lossy(),
            model().0,
            model().1,
            std::env::var("YAVS_LLVM_ARGS")
                .as_deref()
                .unwrap_or(CODEGEN_ARGS),
            std::env::var("YAVS_LLVM_PIPELINE").unwrap_or_default(),
        )
    }
}

fn init() {
    static INIT: Once = Once::new();
    INIT.call_once(|| {
        // SAFETY: plain one-time LLVM initialisation.
        unsafe {
            let dev = std::env::var("YAVS_LLVM_ARGS");
            let a = dev.as_deref().unwrap_or(CODEGEN_ARGS);
            let mut v: Vec<CString> = vec![CString::new("yavs").unwrap()];
            v.extend(a.split_whitespace().map(|x| CString::new(x).unwrap()));
            let p: Vec<*const c_char> = v.iter().map(|x| x.as_ptr()).collect();
            llvm_sys::support::LLVMParseCommandLineOptions(p.len() as i32, p.as_ptr(), ptr::null());
            assert_eq!(LLVM_InitializeNativeTarget(), 0, "no native LLVM target");
            assert_eq!(
                LLVM_InitializeNativeAsmPrinter(),
                0,
                "no native asm printer"
            );
        }
    });
}

/// The portable CPU baseline the JIT targets (design rule 5, sim1 #887/#1311).
/// Never the host CPU, so code behaves the same on every machine.
pub fn portable_cpu() -> &'static CStr {
    if cfg!(target_arch = "x86_64") {
        c"x86-64-v2"
    } else {
        c"generic"
    }
}

/// The most LLVM instructions (before optimisation) a module may have to get the full `O2`
/// pipeline. The generator asks for it only for compute loops (`optimize_as`); everything else
/// (library and testbench functions, whose time goes to what the interpreter's own functions
/// do) gets `LEAN`: it costs a third of `default<O2>` and the compiled code is as fast within
/// the noise (#432).
const FULL_MAX: usize = 3000;

/// Scalar clean-up of the frame register traffic and the guards, no loop, inlining or
/// vectorisation passes: promote, common subexpressions, one combine, branch folding.
const LEAN: &CStr =
    c"function(sroa,early-cse,instcombine<max-iterations=1;no-verify-fixpoint>,simplifycfg,adce)";

/// `LEAN` in two runs (`optimize_as`): the part before `instcombine`, then the rest. The big
/// allocas that are left after `sroa` are hidden from the second run (`hide_allocas`).
const LEAN_A: &CStr = c"function(sroa,early-cse)";
const LEAN_B: &CStr =
    c"function(instcombine<max-iterations=1;no-verify-fixpoint>,simplifycfg,adce)";
const LEAN_HUGE_B: &CStr = c"function(instcombine<max-iterations=1;no-verify-fixpoint>,dce)";

/// An alloca with at least this many users is hidden from `instcombine`.
const HIDE_MIN: usize = 32;

/// `LEAN` without the two passes whose time grows faster than the module for the modules that
/// are all one function: `simplifycfg` (66 s) and `adce` (31 s) of the 115 s the 7 million LLVM
/// instructions of a process of 147 000 IR instructions took, `dce` instead of them: 12 s, the
/// code 6 percent larger (#630).
const LEAN_HUGE: &CStr =
    c"function(sroa,early-cse,instcombine<max-iterations=1;no-verify-fixpoint>,dce)";

/// The module size from which `LEAN_HUGE` stands for `LEAN` (a process of about 5 000 IR
/// instructions): below it the two passes cost less than 10 percent of the time of the module.
const HUGE_MIN: usize = 100_000;

impl OptLevel {
    fn pipeline(self) -> &'static CStr {
        match self {
            OptLevel::O0 => c"default<O0>",
            OptLevel::O1 => c"default<O1>",
            OptLevel::O2 => c"default<O2>",
            OptLevel::Fast | OptLevel::FastRa => LEAN,
        }
    }

    fn codegen(self) -> LLVMCodeGenOptLevel {
        match self {
            OptLevel::O0 => LLVMCodeGenOptLevel::LLVMCodeGenLevelNone,
            OptLevel::O1 => LLVMCodeGenOptLevel::LLVMCodeGenLevelLess,
            OptLevel::O2 => LLVMCodeGenOptLevel::LLVMCodeGenLevelDefault,
            OptLevel::Fast => LLVMCodeGenOptLevel::LLVMCodeGenLevelNone,
            OptLevel::FastRa => LLVMCodeGenOptLevel::LLVMCodeGenLevelLess,
        }
    }
}

/// The code model and relocation mode of the generated code. With the code arena (code, stubs
/// and data within 2 GiB of each other, #679) the small model with position independent code:
/// 32-bit displacements everywhere, a call of a runtime callback is `call [rip+got]` (the
/// callbacks are marked `nonlazybind`, `Gen::effects`), not a 10-byte `movabs` and `call rax`.
/// Without the arena (and with `YAVS_LLVM_CODEMODEL=large`) the JIT default, which spells out
/// every address.
fn model() -> (LLVMRelocMode, LLVMCodeModel) {
    static SMALL: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    let small = *SMALL.get_or_init(|| {
        crate::arena::enabled() && !std::env::var("YAVS_LLVM_CODEMODEL").is_ok_and(|v| v == "large")
    });
    if small {
        (
            LLVMRelocMode::LLVMRelocPIC,
            LLVMCodeModel::LLVMCodeModelSmall,
        )
    } else {
        (
            LLVMRelocMode::LLVMRelocDefault,
            LLVMCodeModel::LLVMCodeModelJITDefault,
        )
    }
}

const LAUNDER: &CStr = c"llvm.launder.invariant.group";

/// Puts every alloca of at least `HIDE_MIN` users behind `llvm.launder.invariant.group`, so that
/// the passes see a call result instead of an alloca (`instcombine` re-examines all users of an
/// alloca each time one of them is edited). Returns whether any was hidden; `unhide_allocas`
/// takes the calls out again.
unsafe fn hide_allocas(module: LLVMModuleRef) -> bool {
    // SAFETY: plain walks over a module that is finished; the calls are made at the alloca and
    // take over its uses.
    unsafe {
        let ctx = LLVMGetModuleContext(module);
        let ptr = LLVMPointerTypeInContext(ctx, 0);
        let mut found = Vec::new();
        let mut f = LLVMGetFirstFunction(module);
        while !f.is_null() {
            if LLVMCountBasicBlocks(f) > 0 {
                let mut i = LLVMGetFirstInstruction(LLVMGetEntryBasicBlock(f));
                while !i.is_null() {
                    if !LLVMIsAAllocaInst(i).is_null() {
                        let (mut n, mut u) = (0, LLVMGetFirstUse(i));
                        while !u.is_null() && n < HIDE_MIN {
                            (n, u) = (n + 1, LLVMGetNextUse(u));
                        }
                        if n >= HIDE_MIN {
                            found.push(i);
                        }
                    }
                    i = LLVMGetNextInstruction(i);
                }
            }
            f = LLVMGetNextFunction(f);
        }
        if found.is_empty() {
            return false;
        }
        let id = LLVMLookupIntrinsicID(LAUNDER.as_ptr(), LAUNDER.count_bytes());
        assert_ne!(id, 0, "unknown intrinsic {LAUNDER:?}");
        let mut tys = [ptr];
        let decl = LLVMGetIntrinsicDeclaration(module, id, tys.as_mut_ptr(), 1);
        let fty = LLVMIntrinsicGetType(ctx, id, tys.as_mut_ptr(), 1);
        let bld = LLVMCreateBuilderInContext(ctx);
        for a in found {
            LLVMPositionBuilderBefore(bld, LLVMGetNextInstruction(a));
            let mut args = [a];
            let call = LLVMBuildCall2(bld, fty, decl, args.as_mut_ptr(), 1, c"".as_ptr());
            LLVMReplaceAllUsesWith(a, call);
            LLVMSetOperand(call, 0, a);
        }
        LLVMDisposeBuilder(bld);
        true
    }
}

/// Takes the calls of `hide_allocas` out again.
unsafe fn unhide_allocas(module: LLVMModuleRef) {
    // SAFETY: the calls are the users of the declaration; each is replaced by its operand.
    unsafe {
        let decl = LLVMGetNamedFunction(module, c"llvm.launder.invariant.group.p0".as_ptr());
        if decl.is_null() {
            return;
        }
        let mut calls = Vec::new();
        let mut u = LLVMGetFirstUse(decl);
        while !u.is_null() {
            calls.push(LLVMGetUser(u));
            u = LLVMGetNextUse(u);
        }
        for c in calls {
            LLVMReplaceAllUsesWith(c, LLVMGetOperand(c, 0));
            LLVMInstructionEraseFromParent(c);
        }
    }
}

/// `and (load i64 p), 0xffffffff` as `zext (load i32 p)`, after the passes (they forward a store
/// to a load of the same bytes, which a narrower load would block): a 4-byte load zero-extends by
/// itself, where the unoptimised code generator makes a 10-byte `movabs` and an `and` of the
/// mask (the low half of a fat address is read this way thousands of times in a large process).
unsafe fn narrow_masks(module: LLVMModuleRef) {
    // SAFETY: plain walks over a module that is finished (the generator is done with it); the
    // loads replaced have no other user.
    unsafe {
        let ctx = LLVMGetModuleContext(module);
        let (i32t, i64t) = (LLVMInt32TypeInContext(ctx), LLVMInt64TypeInContext(ctx));
        let kinds = [
            LLVMGetMDKindIDInContext(ctx, c"alias.scope".as_ptr(), 11),
            LLVMGetMDKindIDInContext(ctx, c"noalias".as_ptr(), 7),
        ];
        let mut found = Vec::new();
        let mut fv = LLVMGetFirstFunction(module);
        while !fv.is_null() {
            let mut b = LLVMGetFirstBasicBlock(fv);
            while !b.is_null() {
                let mut i = LLVMGetFirstInstruction(b);
                while !i.is_null() {
                    if LLVMGetInstructionOpcode(i) == llvm_sys::LLVMOpcode::LLVMAnd {
                        let (x, y) = (LLVMGetOperand(i, 0), LLVMGetOperand(i, 1));
                        let (ld, k) = if LLVMIsAConstantInt(y).is_null() {
                            (y, x)
                        } else {
                            (x, y)
                        };
                        if !LLVMIsAConstantInt(k).is_null()
                            && LLVMConstIntGetZExtValue(k) == 0xffff_ffff
                            && LLVMTypeOf(ld) == i64t
                            && !LLVMIsALoadInst(ld).is_null()
                            && LLVMGetVolatile(ld) == 0
                            && LLVMGetOrdering(ld)
                                == llvm_sys::LLVMAtomicOrdering::LLVMAtomicOrderingNotAtomic
                        {
                            let u = LLVMGetFirstUse(ld);
                            if !u.is_null() && LLVMGetNextUse(u).is_null() {
                                found.push((i, ld));
                            }
                        }
                    }
                    i = LLVMGetNextInstruction(i);
                }
                b = LLVMGetNextBasicBlock(b);
            }
            fv = LLVMGetNextFunction(fv);
        }
        if found.is_empty() {
            return;
        }
        let bld = LLVMCreateBuilderInContext(ctx);
        for (and, ld) in found {
            LLVMPositionBuilderBefore(bld, ld);
            let n = LLVMBuildLoad2(bld, i32t, LLVMGetOperand(ld, 0), c"".as_ptr());
            LLVMSetAlignment(n, LLVMGetAlignment(ld));
            for k in kinds {
                let m = LLVMGetMetadata(ld, k);
                if !m.is_null() {
                    LLVMSetMetadata(n, k, m);
                }
            }
            let z = LLVMBuildZExt(bld, n, i64t, c"".as_ptr());
            LLVMReplaceAllUsesWith(and, z);
            LLVMInstructionEraseFromParent(and);
            LLVMInstructionEraseFromParent(ld);
        }
        LLVMDisposeBuilder(bld);
    }
}

/// Gives each block its own copy of the address arithmetic it uses (`getelementptr` with constant
/// indices, off a base available there), after the passes: they merge the copies of a block with
/// the first one in a dominating block (EarlyCSE), and then the address is a value alive across
/// blocks, which the unoptimised code generator keeps in a stack slot (a store at the definition,
/// a load at every use) and cannot fold into the load or store that uses it (it does not look for
/// an address in another block). The copy folds into the access: no register, no reload.
unsafe fn remat_geps(module: LLVMModuleRef) {
    use llvm_sys::LLVMOpcode;
    use std::collections::HashMap;
    // The value to use in `blk` in place of `v`: a copy of `v` made before `at` when `v` is such an
    // address from another block (its base made local the same way).
    unsafe fn local(
        b: LLVMBuilderRef,
        blk: LLVMBasicBlockRef,
        at: LLVMValueRef,
        v: LLVMValueRef,
        made: &mut HashMap<(usize, usize), LLVMValueRef>,
    ) -> LLVMValueRef {
        // SAFETY: values and blocks of one live function; a clone is built before `at`.
        unsafe {
            if LLVMIsAGetElementPtrInst(v).is_null() || LLVMGetInstructionParent(v) == blk {
                return v;
            }
            let n = LLVMGetNumOperands(v) as u32;
            if (1..n).any(|k| LLVMIsAConstantInt(LLVMGetOperand(v, k)).is_null()) {
                return v;
            }
            if let Some(&c) = made.get(&(blk as usize, v as usize)) {
                return c;
            }
            let base = local(b, blk, at, LLVMGetOperand(v, 0), made);
            let mut idx: Vec<LLVMValueRef> = (1..n).map(|k| LLVMGetOperand(v, k)).collect();
            LLVMPositionBuilderBefore(b, at);
            let g = LLVMBuildGEP2(
                b,
                LLVMGetGEPSourceElementType(v),
                base,
                idx.as_mut_ptr(),
                idx.len() as u32,
                c"".as_ptr(),
            );
            made.insert((blk as usize, v as usize), g);
            g
        }
    }
    // SAFETY: plain walks over a module that is finished; operands are replaced by equal values.
    unsafe {
        let ctx = LLVMGetModuleContext(module);
        let bld = LLVMCreateBuilderInContext(ctx);
        let mut f = LLVMGetFirstFunction(module);
        while !f.is_null() {
            let mut made = HashMap::new();
            let mut gone = Vec::new();
            let mut bb = LLVMGetFirstBasicBlock(f);
            while !bb.is_null() {
                let mut i = LLVMGetFirstInstruction(bb);
                while !i.is_null() {
                    let next = LLVMGetNextInstruction(i);
                    let op = LLVMGetInstructionOpcode(i);
                    // (A phi's operand belongs to the end of its predecessor; a copy there is no gain.)
                    if op != LLVMOpcode::LLVMPHI && op != LLVMOpcode::LLVMGetElementPtr {
                        // (The copies carry the user's line in a build with line tables.)
                        let loc = llvm_sys::debuginfo::LLVMInstructionGetDebugLoc(i);
                        llvm_sys::core::LLVMSetCurrentDebugLocation2(bld, loc);
                        for k in 0..LLVMGetNumOperands(i) as u32 {
                            let v = LLVMGetOperand(i, k);
                            let w = local(bld, bb, i, v, &mut made);
                            if w != v {
                                LLVMSetOperand(i, k, w);
                                gone.push(v);
                            }
                        }
                    }
                    i = next;
                }
                bb = LLVMGetNextBasicBlock(bb);
            }
            // The originals nothing uses any more, a chain of them from the last.
            let mut erased = std::collections::HashSet::new();
            while let Some(v) = gone.pop() {
                if !erased.contains(&(v as usize))
                    && !LLVMIsAGetElementPtrInst(v).is_null()
                    && LLVMGetFirstUse(v).is_null()
                {
                    let base = LLVMGetOperand(v, 0);
                    erased.insert(v as usize);
                    LLVMInstructionEraseFromParent(v);
                    gone.push(base);
                }
            }
            f = LLVMGetNextFunction(f);
        }
        LLVMDisposeBuilder(bld);
    }
}

/// Marks every function `optnone noinline`: the machine passes that `Less` adds over `None` (block
/// placement, tail merging, peepholes) skip them, and the instruction selection stays the fast one,
/// so `OptLevel::FastRa` differs from `Fast` in the register allocator alone (#715).
unsafe fn no_opt(module: LLVMModuleRef) {
    // SAFETY: plain walk over a module that is finished; attributes of a known kind are added.
    unsafe {
        let ctx = LLVMGetModuleContext(module);
        let kinds = [c"optnone", c"noinline"].map(|n| {
            let k = LLVMGetEnumAttributeKindForName(n.as_ptr(), n.to_bytes().len());
            LLVMCreateEnumAttribute(ctx, k, 0)
        });
        let mut f = LLVMGetFirstFunction(module);
        while !f.is_null() {
            if LLVMCountBasicBlocks(f) > 0 {
                for a in kinds {
                    LLVMAddAttributeAtIndex(f, llvm_sys::LLVMAttributeFunctionIndex, a);
                }
            }
            f = LLVMGetNextFunction(f);
        }
    }
}

/// Whether the generated code is the small model (calls to the runtime go through the GOT).
pub(crate) fn small_model() -> bool {
    model().1 != LLVMCodeModel::LLVMCodeModelJITDefault
}

/// A target machine for the host triple with the portable CPU.
fn target_machine(opt: OptLevel) -> Result<LLVMTargetMachineRef> {
    init();
    // SAFETY: all strings are owned LLVM messages or static C strings.
    unsafe {
        let triple = LLVMGetDefaultTargetTriple();
        let mut target = ptr::null_mut();
        let mut msg = ptr::null_mut();
        if LLVMGetTargetFromTriple(triple, &mut target, &mut msg) != 0 {
            LLVMDisposeMessage(triple);
            return Err(Error(take_message(msg)));
        }
        let (reloc, code) = model();
        let tm = LLVMCreateTargetMachine(
            target,
            triple,
            portable_cpu().as_ptr(),
            c"".as_ptr(),
            opt.codegen(),
            reloc,
            code,
        );
        LLVMDisposeMessage(triple);
        if tm.is_null() {
            return Err(Error("cannot create target machine".into()));
        }
        if opt == OptLevel::FastRa {
            LLVMSetTargetMachineFastISel(tm, 1);
        }
        Ok(tm)
    }
}

/// An LLVM module that owns its own context, so it can be built on any
/// thread and handed to the JIT as one unit.
pub struct Module {
    ctx: LLVMContextRef,
    module: LLVMModuleRef,
}

// SAFETY: the module and its context are only touched through `&mut self` or
// by value; an LLVMContext may move between threads when used by one at a time.
unsafe impl Send for Module {}

/// Debug aid: with `YAVS_LLVM_DUMP=<dir>` every optimised module is written there as
/// `<n>-pre.ll` / `<n>-post.ll` (before and after the pass pipeline).
unsafe fn dump(m: LLVMModuleRef, tag: &str) {
    use std::sync::atomic::{AtomicUsize, Ordering};
    static N: AtomicUsize = AtomicUsize::new(0);
    let Some(dir) = std::env::var_os("YAVS_LLVM_DUMP") else {
        return;
    };
    let n = if tag == "pre" {
        N.fetch_add(1, Ordering::Relaxed)
    } else {
        N.load(Ordering::Relaxed) - 1
    };
    let path = std::path::Path::new(&dir).join(format!("{n}-{tag}.ll"));
    if let Ok(c) = std::ffi::CString::new(path.to_string_lossy().as_bytes()) {
        let mut err: *mut c_char = std::ptr::null_mut();
        // SAFETY: the module is live; a message, if any, is freed.
        unsafe {
            if LLVMPrintModuleToFile(m, c.as_ptr(), &mut err) != 0 {
                LLVMDisposeMessage(err);
            }
        }
    }
}

impl Module {
    pub fn new(name: &str) -> Module {
        crate::lazy::ensure().expect("LLVM library");
        let name = CString::new(name).expect("module name has NUL");
        // SAFETY: fresh context and module owned by the returned value.
        unsafe {
            let ctx = LLVMContextCreate();
            let module = LLVMModuleCreateWithNameInContext(name.as_ptr(), ctx);
            Module { ctx, module }
        }
    }

    pub fn context(&self) -> LLVMContextRef {
        self.ctx
    }

    pub fn raw(&self) -> LLVMModuleRef {
        self.module
    }

    /// Runs the verifier.
    pub fn verify(&self) -> Result<()> {
        use llvm_sys::analysis::*;
        let mut msg = ptr::null_mut();
        // SAFETY: module is live; msg is an owned LLVM message.
        unsafe {
            let bad = LLVMVerifyModule(
                self.module,
                LLVMVerifierFailureAction::LLVMReturnStatusAction,
                &mut msg,
            );
            let text = take_message(msg);
            if bad != 0 { Err(Error(text)) } else { Ok(()) }
        }
    }

    /// Number of instructions in all function bodies.
    pub fn instruction_count(&self) -> usize {
        let mut n = 0;
        // SAFETY: plain iteration over a live module.
        unsafe {
            let mut f = LLVMGetFirstFunction(self.module);
            while !f.is_null() {
                let mut bb = LLVMGetFirstBasicBlock(f);
                while !bb.is_null() {
                    let mut i = LLVMGetFirstInstruction(bb);
                    while !i.is_null() {
                        n += 1;
                        i = LLVMGetNextInstruction(i);
                    }
                    bb = LLVMGetNextBasicBlock(bb);
                }
                f = LLVMGetNextFunction(f);
            }
        }
        n
    }

    /// Sets triple and data layout, then runs `default<On>` through the new
    /// pass manager (`LLVMRunPasses`).
    pub fn optimize(&mut self, opt: OptLevel) -> Result<()> {
        self.optimize_as(opt, true)
    }

    /// As `optimize`; at `O2` the full pipeline only when `full` and the module is small
    /// (`FULL_MAX`), else `LEAN`.
    pub fn optimize_as(&mut self, opt: OptLevel, full: bool) -> Result<()> {
        let tm = target_machine(opt)?;
        // SAFETY: tm and module are live; everything created here is disposed.
        unsafe {
            let triple = LLVMGetTargetMachineTriple(tm);
            LLVMSetTarget(self.module, triple);
            LLVMDisposeMessage(triple);
            let dl = LLVMCreateTargetDataLayout(tm);
            LLVMSetModuleDataLayout(self.module, dl);
            LLVMDisposeTargetData(dl);
            dump(self.module, "pre");
            let opts = LLVMCreatePassBuilderOptions();
            let custom = std::env::var("YAVS_LLVM_PIPELINE")
                .ok()
                .and_then(|p| CString::new(p).ok());
            let pipeline = match opt {
                OptLevel::O2 if !full || self.instruction_count() > FULL_MAX => LEAN,
                _ => opt.pipeline(),
            };
            let pipeline = if pipeline == LEAN && self.instruction_count() > HUGE_MIN {
                LEAN_HUGE
            } else {
                pipeline
            };
            let run = |p: &CStr| LLVMRunPasses(self.module, p.as_ptr(), tm, opts);
            let second = if custom.is_some() {
                None
            } else if pipeline == LEAN {
                Some(LEAN_B)
            } else if pipeline == LEAN_HUGE {
                Some(LEAN_HUGE_B)
            } else {
                None
            };
            let err = match second {
                Some(second) => {
                    // `instcombine` walks all users of an alloca whenever one of them changes:
                    // a materialisation buffer with thousands of stores took 90 percent of the
                    // time of the passes (quadratic). The allocas `sroa` leaves are hidden
                    // behind a no-op call for it; the machine code is the same.
                    let err = run(LEAN_A);
                    if err.is_null() {
                        let hidden = hide_allocas(self.module);
                        let err = run(second);
                        if hidden {
                            unhide_allocas(self.module);
                        }
                        err
                    } else {
                        err
                    }
                }
                None => run(custom.as_deref().unwrap_or(pipeline)),
            };
            // The unoptimised code generator does not do this itself.
            if opt.codegen() == LLVMCodeGenOptLevel::LLVMCodeGenLevelNone || opt == OptLevel::FastRa
            {
                narrow_masks(self.module);
                remat_geps(self.module);
            }
            if opt == OptLevel::FastRa {
                no_opt(self.module);
            }
            dump(self.module, "post");
            LLVMDisposePassBuilderOptions(opts);
            LLVMDisposeTargetMachine(tm);
            check(err)
        }
    }

    /// Give the functions `{from}`, `{from}_n` and `{from}_c` (those it has) and the module
    /// itself the names `{to}...`.
    pub fn rename(&mut self, from: &str, to: &str) {
        let set = |v: LLVMValueRef, n: &str| {
            // SAFETY: `v` is a live value of this module; the name is copied.
            unsafe { LLVMSetValueName2(v, n.as_ptr().cast(), n.len()) }
        };
        for suffix in ["", "_n", "_c"] {
            let Ok(old) = CString::new(format!("{from}{suffix}")) else {
                continue;
            };
            // SAFETY: the module is live.
            let f = unsafe { LLVMGetNamedFunction(self.module, old.as_ptr()) };
            if !f.is_null() {
                set(f, &format!("{to}{suffix}"));
            }
        }
        // SAFETY: the module is live; the names are copied.
        unsafe {
            LLVMSetModuleIdentifier(self.module, to.as_ptr().cast(), to.len());
            LLVMSetSourceFileName(self.module, to.as_ptr().cast(), to.len());
        }
    }

    /// The module as bitcode: what its machine code is made from (the key of the object cache).
    pub fn bitcode(&self) -> Vec<u8> {
        // SAFETY: the module is live; the buffer is copied and disposed.
        unsafe {
            take_buffer(llvm_sys::bit_writer::LLVMWriteBitcodeToMemoryBuffer(
                self.module,
            ))
        }
    }

    /// The machine code of the module, after `optimize(opt)`, as an object file: what ORC
    /// makes of the module when it is added with `Jit::add` to a JIT made at `opt`.
    pub fn emit(&self, opt: OptLevel) -> Result<Vec<u8>> {
        let tm = target_machine(opt)?;
        let mut msg = ptr::null_mut();
        let mut buf = ptr::null_mut();
        // SAFETY: tm and module are live; the message and the buffer are taken over.
        unsafe {
            let bad = LLVMTargetMachineEmitToMemoryBuffer(
                tm,
                self.module,
                LLVMCodeGenFileType::LLVMObjectFile,
                &mut msg,
                &mut buf,
            );
            LLVMDisposeTargetMachine(tm);
            if bad != 0 {
                return Err(Error(take_message(msg)));
            }
            Ok(take_buffer(buf))
        }
    }

    /// Hands the module and its context to ORC.
    fn into_thread_safe(self) -> LLVMOrcThreadSafeModuleRef {
        let me = std::mem::ManuallyDrop::new(self);
        // SAFETY: ownership of ctx and module moves into the TSM. The TSM keeps
        // its own reference to the thread-safe context, so ours is dropped.
        unsafe {
            let tsc = LLVMOrcCreateNewThreadSafeContextFromLLVMContext(me.ctx);
            let tsm = LLVMOrcCreateNewThreadSafeModule(me.module, tsc);
            LLVMOrcDisposeThreadSafeContext(tsc);
            tsm
        }
    }
}

impl Drop for Module {
    fn drop(&mut self) {
        // SAFETY: we own both.
        unsafe {
            LLVMDisposeModule(self.module);
            LLVMContextDispose(self.ctx);
        }
    }
}

/// An ORC LLJIT instance targeting the portable CPU.
///
/// Compiled code lives as long as the `Jit`. All adds and lookups are meant
/// to happen on one thread (the compile thread); function pointers it
/// returns may be called from any thread.
pub struct Jit {
    raw: LLVMOrcLLJITRef,
    /// Errors the execution session reports out of band, such as the
    /// unresolved symbols behind a failed materialization. `lookup` attaches
    /// them to its error so the cause is never lost.
    session_errors: Box<Mutex<Vec<String>>>,
    /// Code compiled for a module by its key (`lower::compile_one`): another function whose
    /// module is the same up to its names is that code.
    pub(crate) twins: std::collections::HashMap<String, (usize, usize, usize)>,
}

extern "C" fn report_session_error(ctx: *mut c_void, err: LLVMErrorRef) {
    // SAFETY: ctx is the `session_errors` box of a live Jit.
    let errors = unsafe { &*(ctx as *const Mutex<Vec<String>>) };
    if let Err(e) = check(err) {
        errors.lock().unwrap_or_else(|p| p.into_inner()).push(e.0);
    }
}

// SAFETY: LLJIT has no thread affinity; `&mut self` serialises adds/lookups.
unsafe impl Send for Jit {}

impl Jit {
    /// Creates a JIT whose main dylib defines `symbols` as absolute addresses
    /// (the host runtime callbacks). Nothing is resolved from the process.
    pub fn new(opt: OptLevel, symbols: &[(&str, usize)]) -> Result<Jit> {
        crate::lazy::ensure()?;
        let tm = target_machine(opt)?;
        // SAFETY: the builder takes the JTMB, CreateLLJIT takes the builder,
        // AbsoluteSymbols takes the interned names, Define takes the MU on success.
        unsafe {
            let jtmb = LLVMOrcJITTargetMachineBuilderCreateFromTargetMachine(tm);
            let builder = LLVMOrcCreateLLJITBuilder();
            LLVMOrcLLJITBuilderSetJITTargetMachineBuilder(builder, jtmb);
            #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
            if crate::arena::enabled() {
                LLVMOrcLLJITBuilderSetObjectLinkingLayerCreator(
                    builder,
                    crate::arena::create_layer,
                    ptr::null_mut(),
                );
            }
            let mut raw = ptr::null_mut();
            check(LLVMOrcCreateLLJIT(&mut raw, builder))?;
            let jit = Jit {
                raw,
                session_errors: Box::default(),
                twins: Default::default(),
            };
            LLVMOrcExecutionSessionSetErrorReporter(
                LLVMOrcLLJITGetExecutionSession(raw),
                report_session_error,
                &*jit.session_errors as *const _ as *mut c_void,
            );
            if !symbols.is_empty() {
                let mut pairs: Vec<LLVMOrcCSymbolMapPair> = symbols
                    .iter()
                    .map(|&(name, addr)| {
                        let name = CString::new(name).expect("symbol name has NUL");
                        LLVMOrcCSymbolMapPair {
                            Name: LLVMOrcLLJITMangleAndIntern(raw, name.as_ptr()),
                            Sym: LLVMJITEvaluatedSymbol {
                                Address: addr as u64,
                                Flags: LLVMJITSymbolFlags {
                                    GenericFlags: LLVMJITSymbolGenericFlags::LLVMJITSymbolGenericFlagsExported
                                        as u8
                                        | LLVMJITSymbolGenericFlags::LLVMJITSymbolGenericFlagsCallable
                                            as u8,
                                    TargetFlags: 0,
                                },
                            },
                        }
                    })
                    .collect();
                let mu = LLVMOrcAbsoluteSymbols(pairs.as_mut_ptr(), pairs.len());
                let err = LLVMOrcJITDylibDefine(LLVMOrcLLJITGetMainJITDylib(raw), mu);
                if !err.is_null() {
                    LLVMOrcDisposeMaterializationUnit(mu);
                }
                check(err)?;
            }
            Ok(jit)
        }
    }

    /// Adds a module to the main dylib. Code is generated lazily, on the first
    /// lookup of one of its symbols.
    pub fn add(&mut self, module: Module) -> Result<()> {
        let tsm = module.into_thread_safe();
        // SAFETY: on success LLJIT owns the TSM; on failure we still do.
        unsafe {
            let jd = LLVMOrcLLJITGetMainJITDylib(self.raw);
            let err = LLVMOrcLLJITAddLLVMIRModule(self.raw, jd, tsm);
            if !err.is_null() {
                LLVMOrcDisposeThreadSafeModule(tsm);
            }
            check(err)
        }
    }

    /// Adds an object file (`Module::emit`, or one the object cache kept) to the main dylib.
    /// ORC links it when one of its symbols is looked up: its references to the runtime
    /// callbacks are resolved by name to this JIT's absolute symbols.
    pub fn add_object(&mut self, obj: &[u8]) -> Result<()> {
        // SAFETY: the buffer is a copy that LLJIT takes over, also on failure.
        unsafe {
            let buf = LLVMCreateMemoryBufferWithMemoryRangeCopy(
                obj.as_ptr().cast::<c_char>(),
                obj.len(),
                c"yavs-object".as_ptr(),
            );
            let jd = LLVMOrcLLJITGetMainJITDylib(self.raw);
            check(LLVMOrcLLJITAddObjectFile(self.raw, jd, buf))
        }
    }

    /// Looks up an (unmangled) symbol, compiling and linking it if needed.
    pub fn lookup(&mut self, name: &str) -> Result<usize> {
        let name = CString::new(name).expect("symbol name has NUL");
        let mut addr = 0;
        // SAFETY: LLJIT mangles the name itself and does not keep it.
        let res = check(unsafe { LLVMOrcLLJITLookup(self.raw, &mut addr, name.as_ptr()) });
        let mut errors = self
            .session_errors
            .lock()
            .unwrap_or_else(|p| p.into_inner());
        if res.is_ok() {
            perf_map(&name, addr as usize);
        }
        match res {
            Ok(()) if errors.is_empty() => Ok(addr as usize),
            Ok(()) => Err(Error(errors.drain(..).collect::<Vec<_>>().join("; "))),
            Err(Error(msg)) => {
                let causes = errors.drain(..).collect::<Vec<_>>();
                Err(Error(
                    causes
                        .into_iter()
                        .chain([msg])
                        .collect::<Vec<_>>()
                        .join("; "),
                ))
            }
        }
    }

    /// The triple the JIT generates code for.
    pub fn triple(&self) -> String {
        // SAFETY: the string is owned by the JIT.
        unsafe { CStr::from_ptr(LLVMOrcLLJITGetTripleString(self.raw)) }
            .to_string_lossy()
            .into_owned()
    }
}

impl Drop for Jit {
    fn drop(&mut self) {
        // SAFETY: we own the JIT. Drop cannot return the error, so it is
        // reported rather than ignored (fail early, fail clearly).
        if let Err(e) = check(unsafe { LLVMOrcDisposeLLJIT(self.raw) }) {
            eprintln!("error: tearing down the LLVM JIT failed: {e}");
        }
    }
}

/// Profiling aid: with `YAVS_PERFMAP` set, every looked-up symbol is listed in `/tmp/perf-<pid>.map`
/// (the file `perf report` reads for code it does not know), so compiled functions show under
/// their symbol (`f<id>`, `f<id>_n`) instead of as anonymous JIT addresses. A symbol's size is the
/// distance to the next one.
fn perf_map(name: &CStr, addr: usize) {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    static MAP: Mutex<Vec<(usize, String)>> = Mutex::new(Vec::new());
    if !*ON.get_or_init(|| std::env::var_os("YAVS_PERFMAP").is_some()) {
        return;
    }
    let mut m = MAP.lock().unwrap_or_else(|p| p.into_inner());
    m.push((addr, name.to_string_lossy().into_owned()));
    m.sort_unstable();
    let mut out = String::new();
    for (i, (a, n)) in m.iter().enumerate() {
        let end = m.get(i + 1).map_or(a + 0x4000, |x| x.0);
        out += &format!("{a:x} {:x} {n}\n", (end - a).min(0x10_0000));
    }
    let _ = std::fs::write(format!("/tmp/perf-{}.map", std::process::id()), out);
}

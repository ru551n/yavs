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

fn init() {
    static INIT: Once = Once::new();
    INIT.call_once(|| {
        // SAFETY: plain one-time LLVM initialisation.
        unsafe {
            assert_eq!(LLVM_InitializeNativeTarget(), 0, "no native LLVM target");
            assert_eq!(
                LLVM_InitializeNativeAsmPrinter(),
                0,
                "no native asm printer"
            );
        }
    });
}

/// The portable CPU baseline the JIT targets (design rule 5, NVC #887/#1311).
/// Never the host CPU, so code behaves the same on every machine.
pub fn portable_cpu() -> &'static CStr {
    if cfg!(target_arch = "x86_64") {
        c"x86-64-v2"
    } else {
        c"generic"
    }
}

/// Optimisation level for the new pass manager and for codegen.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OptLevel {
    O0,
    O1,
    O2,
}

impl OptLevel {
    fn pipeline(self) -> &'static CStr {
        match self {
            OptLevel::O0 => c"default<O0>",
            OptLevel::O1 => c"default<O1>",
            OptLevel::O2 => c"default<O2>",
        }
    }

    fn codegen(self) -> LLVMCodeGenOptLevel {
        match self {
            OptLevel::O0 => LLVMCodeGenOptLevel::LLVMCodeGenLevelNone,
            OptLevel::O1 => LLVMCodeGenOptLevel::LLVMCodeGenLevelLess,
            OptLevel::O2 => LLVMCodeGenOptLevel::LLVMCodeGenLevelDefault,
        }
    }
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
        let tm = LLVMCreateTargetMachine(
            target,
            triple,
            portable_cpu().as_ptr(),
            c"".as_ptr(),
            opt.codegen(),
            LLVMRelocMode::LLVMRelocDefault,
            LLVMCodeModel::LLVMCodeModelJITDefault,
        );
        LLVMDisposeMessage(triple);
        if tm.is_null() {
            return Err(Error("cannot create target machine".into()));
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

impl Module {
    pub fn new(name: &str) -> Module {
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
        let tm = target_machine(opt)?;
        // SAFETY: tm and module are live; everything created here is disposed.
        unsafe {
            let triple = LLVMGetTargetMachineTriple(tm);
            LLVMSetTarget(self.module, triple);
            LLVMDisposeMessage(triple);
            let dl = LLVMCreateTargetDataLayout(tm);
            LLVMSetModuleDataLayout(self.module, dl);
            LLVMDisposeTargetData(dl);
            let opts = LLVMCreatePassBuilderOptions();
            let err = LLVMRunPasses(self.module, opt.pipeline().as_ptr(), tm, opts);
            LLVMDisposePassBuilderOptions(opts);
            LLVMDisposeTargetMachine(tm);
            check(err)
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
        let tm = target_machine(opt)?;
        // SAFETY: the builder takes the JTMB, CreateLLJIT takes the builder,
        // AbsoluteSymbols takes the interned names, Define takes the MU on success.
        unsafe {
            let jtmb = LLVMOrcJITTargetMachineBuilderCreateFromTargetMachine(tm);
            let builder = LLVMOrcCreateLLJITBuilder();
            LLVMOrcLLJITBuilderSetJITTargetMachineBuilder(builder, jtmb);
            let mut raw = ptr::null_mut();
            check(LLVMOrcCreateLLJIT(&mut raw, builder))?;
            let jit = Jit {
                raw,
                session_errors: Box::default(),
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

//! A program read with `lazy_bodies` decodes a function's body when it is first handed out and
//! is the same program as one read eagerly, and writes back identically.

use yavs_diag::{Buffer, FileId, Span, sharing};
use yavs_ir::*;

fn sp() -> Span {
    Span::new(FileId(0), 0, 1)
}

fn program() -> (Program, FuncId, FuncId) {
    let mut pb = ProgramBuilder::new();
    let mut ids = Vec::new();
    for (name, v) in [("f", 1), ("g", 2)] {
        let f = pb.declare(name, FuncKind::Function, None);
        let mut fb = pb.define(f);
        fb.stmt(StmtKind::Null, sp());
        let r = fb.reg(RegTy::I64);
        fb.emit(Inst::Const { dst: r, v });
        fb.emit(Inst::Ret { v: Some(r) });
        fb.finish();
        ids.push(f);
    }
    (pb.finish().expect("well formed"), ids[0], ids[1])
}

#[test]
fn lazy_program_equals_eager_and_writes_back_the_same() {
    let (p, f, g) = program();
    let bytes = postcard::to_stdvec(&p).expect("serialize");
    let buf = Buffer::new(bytes.clone());
    let lazy: Program = sharing(&buf, || lazy_bodies(|| postcard::from_bytes(&buf))).expect("lazy");
    let eager: Program = postcard::from_bytes(&bytes).expect("eager");
    // Undecoded: headers are there, bodies are not.
    let raw = lazy.funcs.get(f).expect("f");
    assert_eq!((raw.name.as_str(), raw.code.len()), ("f", 0));
    assert!(!eager.funcs.get(f).expect("f").code.is_empty());
    // Handed out: the same code, locations and blocks as the eager program.
    for id in [f, g] {
        let (a, b) = (lazy.func(id).expect("lazy"), eager.func(id).expect("eager"));
        assert_eq!(
            (&a.code, &a.loc, &a.operands),
            (&b.code, &b.loc, &b.operands)
        );
        assert_eq!(a.resume, b.resume);
    }
    assert_eq!(lazy.to_string(), eager.to_string());
    // A lazy program that was only partly decoded writes the same bytes, and so does a clone.
    let (partial, _, _) = program();
    let pbytes = postcard::to_stdvec(&partial).expect("serialize");
    let pbuf = Buffer::new(pbytes.clone());
    let again: Program =
        sharing(&pbuf, || lazy_bodies(|| postcard::from_bytes(&pbuf))).expect("lazy");
    again.func(f).expect("f");
    assert_eq!(postcard::to_stdvec(&again).expect("again"), pbytes);
    assert_eq!(postcard::to_stdvec(&again.clone()).expect("clone"), pbytes);
}

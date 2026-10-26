//! std.textio, file objects and std.env natives, run through the kernel and at elaboration.

mod common;

use std::sync::Arc;

use common::*;
use yavs_interp::{Bindings, FILE_OUTPUT, InstData, Runtime};
use yavs_ir::*;
use yavs_kernel::*;

struct Tys {
    int: TyId,
    text: TyId,
    boolean: TyId,
    status: TyId,
}

fn tys(pb: &mut ProgramBuilder) -> Tys {
    let int = integer(pb);
    let ch = pb.ty(Ty::Enum {
        lits: (0..=255u8).map(|c| format!("'{}'", c as char)).collect(),
        logic: None,
    });
    let string = pb.ty(Ty::Array {
        elem: ch,
        elem_desc: None,
        dims: 1,
    });
    let text = pb.ty(Ty::File { elem: string });
    let boolean = pb.ty(Ty::Enum {
        lits: vec!["false".into(), "true".into()],
        logic: Some(LogicKind::Boolean),
    });
    let status = pb.ty(Ty::Enum {
        lits: ["open_ok", "status_error", "name_error", "mode_error"]
            .map(String::from)
            .to_vec(),
        logic: None,
    });
    Tys {
        int,
        text,
        boolean,
        status,
    }
}

fn scratch(name: &str) -> String {
    let d = std::path::Path::new(env!("CARGO_TARGET_TMPDIR")).join("textio");
    std::fs::create_dir_all(&d).unwrap();
    d.join(name).to_str().unwrap().to_string()
}

/// write, writeline, file_close, file_open with status, readline, read with GOOD, endfile,
/// and writeline to OUTPUT, all in one process and one delta.
#[test]
fn write_then_read_back() {
    let path = scratch("roundtrip.txt");
    let _ = std::fs::remove_file(&path);
    let mut pb = ProgramBuilder::new();
    let t = tys(&mut pb);
    let p = pb.declare("p", FuncKind::Process, None);
    {
        let mut fb = pb.define(p);
        let f = fb.slot(4, 4);
        let l = fb.slot(8, 8);
        fb.root(l, RootKind::Handle);
        let good = fb.slot(1, 1);
        let v = fb.slot(8, 8);
        let st = fb.slot(1, 1);
        let s3 = fb.slot(3, 1);
        let a = |fb: &mut FuncBuilder<'_>, off| fb.addr(AddrSpace::Frame, off);
        let call =
            |fb: &mut FuncBuilder<'_>, sym: &str, ty: Option<TyId>, args: &[Reg], ret: bool| {
                native(fb, sym, ty, args, ret)
            };
        // file_open(f, path, write_mode)
        stmt(&mut fb, StmtKind::Call);
        let (fa, name, wm) = (a(&mut fb, f), fat_str(&mut fb, path.as_bytes()), fb.int(1));
        call(
            &mut fb,
            "yavs.implicit.file_open.text",
            Some(t.text),
            &[fa, name, wm],
            false,
        );
        // write(l, 42, right, 5); write(l, " hi", left, 0); writeline(f, l)
        stmt(&mut fb, StmtKind::Call);
        let (la, v42, right, five) = (a(&mut fb, l), fb.int(42), fb.int(0), fb.int(5));
        call(
            &mut fb,
            "yavs.textio.write.line_integer_side_width",
            None,
            &[la, v42, right, five],
            false,
        );
        stmt(&mut fb, StmtKind::Call);
        let (la, hi, left, zero) = (
            a(&mut fb, l),
            fat_str(&mut fb, b" hi"),
            fb.int(1),
            fb.int(0),
        );
        call(
            &mut fb,
            "yavs.textio.write.line_string_side_width",
            None,
            &[la, hi, left, zero],
            false,
        );
        stmt(&mut fb, StmtKind::Call);
        let (fa, la) = (a(&mut fb, f), a(&mut fb, l));
        call(
            &mut fb,
            "yavs.textio.writeline.text_line",
            None,
            &[fa, la],
            false,
        );
        stmt(&mut fb, StmtKind::Call);
        let fa = a(&mut fb, f);
        call(
            &mut fb,
            "yavs.implicit.file_close.text",
            Some(t.text),
            &[fa],
            false,
        );
        // file_open(st, f, path, read_mode); report st
        stmt(&mut fb, StmtKind::Call);
        let (sa, fa, name, rm) = (
            a(&mut fb, st),
            a(&mut fb, f),
            fat_str(&mut fb, path.as_bytes()),
            fb.int(0),
        );
        call(
            &mut fb,
            "yavs.implicit.file_open.text",
            Some(t.text),
            &[sa, fa, name, rm],
            false,
        );
        stmt(&mut fb, StmtKind::Report);
        let s = load(&mut fb, AddrSpace::Frame, st, Width::B1);
        report_image(&mut fb, "status ", t.status, s);
        // readline(f, l); read(l, v, good); report v, good
        stmt(&mut fb, StmtKind::Call);
        let (fa, la) = (a(&mut fb, f), a(&mut fb, l));
        call(
            &mut fb,
            "yavs.textio.readline.text_line",
            None,
            &[fa, la],
            false,
        );
        stmt(&mut fb, StmtKind::Call);
        let (la, va, ga) = (a(&mut fb, l), a(&mut fb, v), a(&mut fb, good));
        call(
            &mut fb,
            "yavs.textio.read.line_integer_boolean",
            None,
            &[la, va, ga],
            false,
        );
        stmt(&mut fb, StmtKind::Report);
        let x = load(&mut fb, AddrSpace::Frame, v, Width::B8);
        report_image(&mut fb, "v ", t.int, x);
        stmt(&mut fb, StmtKind::Report);
        let g = load(&mut fb, AddrSpace::Frame, good, Width::B1);
        report_image(&mut fb, "good ", t.boolean, g);
        // read(l, s3) -> " hi"; a second integer read fails: good = false
        stmt(&mut fb, StmtKind::Call);
        let (la, sa) = (a(&mut fb, l), a(&mut fb, s3));
        let bounds = [fb.int(1), fb.int(3), fb.int(0)];
        call(
            &mut fb,
            "yavs.textio.read.line_string",
            None,
            &[la, sa, bounds[0], bounds[1], bounds[2]],
            false,
        );
        stmt(&mut fb, StmtKind::Report);
        let sa = a(&mut fb, s3);
        let sf = fat_of(&mut fb, sa, 1, 3, false);
        report_with(&mut fb, "s ", sf);
        stmt(&mut fb, StmtKind::Call);
        let (la, va, ga) = (a(&mut fb, l), a(&mut fb, v), a(&mut fb, good));
        call(
            &mut fb,
            "yavs.textio.read.line_integer_boolean",
            None,
            &[la, va, ga],
            false,
        );
        stmt(&mut fb, StmtKind::Report);
        let g = load(&mut fb, AddrSpace::Frame, good, Width::B1);
        report_image(&mut fb, "good ", t.boolean, g);
        stmt(&mut fb, StmtKind::Report);
        let fa = a(&mut fb, f);
        let e = call(
            &mut fb,
            "yavs.implicit.endfile.text",
            Some(t.text),
            &[fa],
            true,
        )
        .unwrap();
        report_image(&mut fb, "endfile ", t.boolean, e);
        // write(l, "to output", ...); writeline(OUTPUT, l)
        stmt(&mut fb, StmtKind::Call);
        let (la, s, right, zero) = (
            a(&mut fb, l),
            fat_str(&mut fb, b"to output"),
            fb.int(0),
            fb.int(0),
        );
        call(
            &mut fb,
            "yavs.textio.write.line_string_side_width",
            None,
            &[la, s, right, zero],
            false,
        );
        stmt(&mut fb, StmtKind::Call);
        let out = fb.slot(4, 4);
        let o = fb.int(FILE_OUTPUT.into());
        store(&mut fb, AddrSpace::Frame, out, o, Width::B4);
        let (oa, la) = (a(&mut fb, out), a(&mut fb, l));
        call(
            &mut fb,
            "yavs.textio.writeline.text_line",
            None,
            &[oa, la],
            false,
        );
        wait_forever(&mut fb);
        fb.finish();
    }
    let mut d = Design::new(pb);
    d.procs = vec![ProcSpec {
        name: "p",
        func: p,
        drives: vec![],
    }];
    let (lines, r) = d.run(Time::ns(1));
    assert_eq!(r, Ok(RunStatus::Ended(EndReason::Idle)));
    assert_eq!(std::fs::read(&path).unwrap(), b"   42 hi\n");
    assert_eq!(
        lines,
        [
            "0 fs +0 p0 Note: status open_ok",
            "0 fs +0 p0 Note: v 42",
            "0 fs +0 p0 Note: good true",
            "0 fs +0 p0 Note: s  hi",
            "0 fs +0 p0 Note: good false",
            "0 fs +0 p0 Note: endfile true",
            "0 fs +0 p0 out: \"to output\\n\"",
        ]
    );
}

/// Opening a missing file without a status is an error naming the file; READ without GOOD
/// on bad text is an error.
#[test]
fn file_and_read_errors() {
    let missing = scratch("does-not-exist.txt");
    let mut pb = ProgramBuilder::new();
    let t = tys(&mut pb);
    let f1 = pb.declare("open_missing", FuncKind::Procedure, None);
    {
        let mut fb = pb.define(f1);
        let f = fb.slot(4, 4);
        stmt(&mut fb, StmtKind::Call);
        let (fa, name, rm) = (
            fb.addr(AddrSpace::Frame, f),
            fat_str(&mut fb, missing.as_bytes()),
            fb.int(0),
        );
        native(
            &mut fb,
            "yavs.implicit.file_open.text",
            Some(t.text),
            &[fa, name, rm],
            false,
        );
        fb.emit(Inst::Ret { v: None });
        fb.finish();
    }
    let f2 = pb.declare("bad_read", FuncKind::Procedure, None);
    {
        let mut fb = pb.define(f2);
        let l = fb.slot(8, 8);
        let v = fb.slot(8, 8);
        stmt(&mut fb, StmtKind::Call);
        let (la, s, r, z) = (
            fb.addr(AddrSpace::Frame, l),
            fat_str(&mut fb, b"xyz"),
            fb.int(0),
            fb.int(0),
        );
        native(
            &mut fb,
            "yavs.textio.write.line_string_side_width",
            None,
            &[la, s, r, z],
            false,
        );
        stmt(&mut fb, StmtKind::Call);
        let (la, va) = (fb.addr(AddrSpace::Frame, l), fb.addr(AddrSpace::Frame, v));
        native(
            &mut fb,
            "yavs.textio.read.line_integer",
            None,
            &[la, va],
            false,
        );
        fb.emit(Inst::Ret { v: None });
        fb.finish();
    }
    let prog = Arc::new(pb.finish().unwrap());
    let rt = Runtime::new(prog, Bindings::default()).unwrap();
    let inst = Arc::new(InstData::default());
    let e = yavs_interp::call(rt.clone(), f1, inst.clone(), &[]).unwrap_err();
    let ErrorKind::Runtime(m) = &e.kind else {
        panic!("{e}")
    };
    assert!(
        m.starts_with("file_open of \"") && m.contains("does-not-exist.txt"),
        "{m}"
    );
    // A failed READ without GOOD is an error report, not a stop (reported to stderr here).
    yavs_interp::call(rt, f2, inst, &[]).unwrap();
}

/// getenv, vhdl_version, dir_createdir, dir_open (items listed), dir_close, dir_deletedir.
#[test]
fn env_natives() {
    let dir = scratch("envdir");
    let _ = std::fs::remove_dir_all(&dir);
    let mut pb = ProgramBuilder::new();
    tys(&mut pb);
    let f = pb.declare("f", FuncKind::Function, None);
    {
        let mut fb = pb.define(f);
        let dslot = fb.slot(16, 8);
        let st = fb.slot(1, 1);
        stmt(&mut fb, StmtKind::Return);
        // dir_createdir(dir, parents => true, st)
        let (p, yes, sa) = (
            fat_str(&mut fb, dir.as_bytes()),
            fb.int(1),
            fb.addr(AddrSpace::Frame, st),
        );
        native(
            &mut fb,
            "yavs.env.dir_createdir.string_boolean_dir_create_status",
            None,
            &[p, yes, sa],
            false,
        );
        let created = load(&mut fb, AddrSpace::Frame, st, Width::B1);
        // dir_open(d, dir) = status_ok, with one item after creating a subdirectory
        let sub = fat_str(&mut fb, format!("{dir}/sub").as_bytes());
        let no = fb.int(0);
        native(
            &mut fb,
            "yavs.env.dir_createdir.string_boolean.r_dir_create_status",
            None,
            &[sub, no],
            true,
        );
        // A record goes to a native as a fat pair, like an array.
        let da = fb.addr(AddrSpace::Frame, dslot);
        let (da, p) = (
            fat_of(&mut fb, da, 0, 0, false),
            fat_str(&mut fb, dir.as_bytes()),
        );
        let opened = native(
            &mut fb,
            "yavs.env.dir_open.directory_string.r_dir_open_status",
            None,
            &[da, p],
            true,
        )
        .unwrap();
        let p = fat_str(&mut fb, dir.as_bytes());
        let isdir = native(
            &mut fb,
            "yavs.env.dir_itemisdir.string.r_boolean",
            None,
            &[p],
            true,
        )
        .unwrap();
        let da = fb.addr(AddrSpace::Frame, dslot);
        let da = fat_of(&mut fb, da, 0, 0, false);
        native(&mut fb, "yavs.env.dir_close.directory", None, &[da], false);
        // dir_deletedir(dir, recursive => true) = status_ok
        let (p, yes) = (fat_str(&mut fb, dir.as_bytes()), fb.int(1));
        let deleted = native(
            &mut fb,
            "yavs.env.dir_deletedir.string_boolean.r_dir_delete_status",
            None,
            &[p, yes],
            true,
        )
        .unwrap();
        // result = created * 1000 + opened * 100 + isdir * 10 + deleted (statuses: 0 ok)
        let k = [1000, 100, 10, 1].map(|k| fb.int(k));
        let mut acc = fb.int(0);
        for (r, k) in [created, opened, isdir, deleted].into_iter().zip(k) {
            let m = bin(&mut fb, IntOp::Mul, r, k);
            acc = bin(&mut fb, IntOp::Add, acc, m);
        }
        fb.emit(Inst::Ret { v: Some(acc) });
        fb.finish();
    }
    let g = pb.declare("g", FuncKind::Function, None);
    {
        let mut fb = pb.define(g);
        stmt(&mut fb, StmtKind::Return);
        let v = native_str(&mut fb, "yavs.env.vhdl_version.void.r_string", &[]);
        let (_, n) = fat_parts(&mut fb, v);
        let name = fat_str(&mut fb, b"YAVS_TEXTIO_TEST_UNSET_VARIABLE");
        let e = native_str(&mut fb, "yavs.env.getenv.string.r_string", &[name]);
        let (_, m) = fat_parts(&mut fb, e);
        let k = fb.int(100);
        let n = bin(&mut fb, IntOp::Mul, n, k);
        let r = bin(&mut fb, IntOp::Add, n, m);
        fb.emit(Inst::Ret { v: Some(r) });
        fb.finish();
    }
    let prog = Arc::new(pb.finish().unwrap());
    let rt = Runtime::new(prog, Bindings::default()).unwrap();
    let inst = Arc::new(InstData::default());
    let r = yavs_interp::call(rt.clone(), f, inst.clone(), &[]).unwrap();
    // created ok, opened ok, isdir true (1), deleted ok.
    assert_eq!(r, Some(10));
    assert!(!std::path::Path::new(&dir).exists());
    // "2008" has 4 characters; the unset variable is "".
    assert_eq!(yavs_interp::call(rt, g, inst, &[]).unwrap(), Some(400));
}

/// bit_vector through WRITE, HWRITE and READ: bits are the '0'/'1' codes of the planes,
/// leftmost element at the high end.
#[test]
fn bit_vectors() {
    let mut pb = ProgramBuilder::new();
    // "1100" in planes: element 0 (rightmost) first.
    let codes = [2u8, 2, 3, 3];
    let src = pb.konst(ConstData::Logic(codes.iter().map(|&c| (1, c)).collect()));
    let f = pb.declare("f", FuncKind::Function, None);
    {
        let mut fb = pb.define(f);
        let l = fb.slot(8, 8);
        let back = fb.slot(32, 8);
        stmt(&mut fb, StmtKind::Return);
        let data = fb.addr(AddrSpace::Const(src), 0);
        let v = fat_of(&mut fb, data, 0, 3, false);
        let (la, r, z) = (fb.addr(AddrSpace::Frame, l), fb.int(0), fb.int(0));
        native(
            &mut fb,
            "yavs.textio.write.line_bit_vector_side_width",
            None,
            &[la, v, r, z],
            false,
        );
        native(
            &mut fb,
            "yavs.textio.hwrite.line_bit_vector_side_width",
            None,
            &[la, v, r, z],
            false,
        );
        // The line is now "1100C"; read the four bits back.
        let ba = fb.addr(AddrSpace::Frame, back);
        let bf = fat_of(&mut fb, ba, 0, 3, false);
        native(
            &mut fb,
            "yavs.textio.read.line_bit_vector",
            None,
            &[la, bf],
            false,
        );
        let n = fb.int(4);
        let eq = fb.reg(RegTy::I64);
        fb.emit(Inst::LEq {
            dst: eq,
            a: ba,
            b: data,
            n,
        });
        // Then the rest of the line must be "C": read a character.
        let ch = fb.slot(1, 1);
        let ca = fb.addr(AddrSpace::Frame, ch);
        native(
            &mut fb,
            "yavs.textio.read.line_character",
            None,
            &[la, ca],
            false,
        );
        let c = load(&mut fb, AddrSpace::Frame, ch, Width::B1);
        let k = fb.int(1000);
        let eq = bin(&mut fb, IntOp::Mul, eq, k);
        let r = bin(&mut fb, IntOp::Add, eq, c);
        fb.emit(Inst::Ret { v: Some(r) });
        fb.finish();
    }
    let prog = Arc::new(pb.finish().unwrap());
    let rt = Runtime::new(prog, Bindings::default()).unwrap();
    let r = yavs_interp::call(rt, f, Arc::new(InstData::default()), &[]).unwrap();
    assert_eq!(r, Some(1000 + u64::from(b'C')));
}

/// N WRITEs and then N READs on one line take linear time: each call used to copy and
/// reallocate the whole line (#196). Timed as a ratio so the machine's speed cancels out.
#[test]
fn long_line_is_linear() {
    // Best of three, so a scheduling hiccup on a loaded machine does not decide.
    let best = |n| (0..3).map(|_| long_line(n)).min().unwrap();
    let (t1, t8) = (best(25_000), best(200_000));
    // Linear is about 8 times, quadratic was about 40.
    assert!(
        t8 < t1 * 20 + std::time::Duration::from_millis(500),
        "{t1:?} then {t8:?}"
    );
}

/// The time `long_line_is_linear` takes for `n` values.
fn long_line(n: i64) -> std::time::Duration {
    let mut pb = ProgramBuilder::new();
    let f = pb.declare("f", FuncKind::Function, None);
    {
        let mut fb = pb.define(f);
        let (l, i, v, sum) = (fb.slot(8, 8), fb.slot(8, 8), fb.slot(8, 8), fb.slot(8, 8));
        stmt(&mut fb, StmtKind::Return);
        let (wr, rd, done) = (fb.block(), fb.block(), fb.block());
        fb.emit(Inst::Br { to: wr });
        // for i in 0 to N - 1: write(l, i, right, 8)
        fb.at(wr);
        let la = fb.addr(AddrSpace::Frame, l);
        let x = load(&mut fb, AddrSpace::Frame, i, Width::B8);
        let (r, w) = (fb.int(0), fb.int(8));
        native(
            &mut fb,
            "yavs.textio.write.line_integer_side_width",
            None,
            &[la, x, r, w],
            false,
        );
        let one = fb.int(1);
        let x = bin(&mut fb, IntOp::Add, x, one);
        store(&mut fb, AddrSpace::Frame, i, x, Width::B8);
        let n = fb.int(n);
        let c = cmp(&mut fb, CmpOp::Lt, x, n);
        fb.emit(Inst::CondBr { c, t: wr, f: rd });
        // N times: read(l, v); sum := sum + v
        fb.at(rd);
        let la = fb.addr(AddrSpace::Frame, l);
        let va = fb.addr(AddrSpace::Frame, v);
        native(
            &mut fb,
            "yavs.textio.read.line_integer",
            None,
            &[la, va],
            false,
        );
        let x = load(&mut fb, AddrSpace::Frame, v, Width::B8);
        let s = load(&mut fb, AddrSpace::Frame, sum, Width::B8);
        let s = bin(&mut fb, IntOp::Add, s, x);
        store(&mut fb, AddrSpace::Frame, sum, s, Width::B8);
        let k = load(&mut fb, AddrSpace::Frame, i, Width::B8);
        let one = fb.int(1);
        let k = bin(&mut fb, IntOp::Sub, k, one);
        store(&mut fb, AddrSpace::Frame, i, k, Width::B8);
        let z = fb.int(0);
        let c = cmp(&mut fb, CmpOp::Gt, k, z);
        fb.emit(Inst::CondBr { c, t: rd, f: done });
        fb.at(done);
        let s = load(&mut fb, AddrSpace::Frame, sum, Width::B8);
        fb.emit(Inst::Ret { v: Some(s) });
        fb.finish();
    }
    let prog = Arc::new(pb.finish().unwrap());
    let rt = Runtime::new(prog, Bindings::default()).unwrap();
    let t = std::time::Instant::now();
    let r = yavs_interp::call(rt, f, Arc::new(InstData::default()), &[]).unwrap();
    assert_eq!(r, Some((n * (n - 1) / 2) as u64));
    t.elapsed()
}

/// `integer` array elements: 4 bytes each (`Ty::elem_size`).
fn ints(vals: &[i32]) -> ConstData {
    ConstData::Bytes(vals.iter().flat_map(|v| v.to_le_bytes()).collect())
}

/// Files of an unconstrained integer array: a read into a shorter object takes what fits,
/// reports it in LENGTH and skips the rest of that record, so the next read starts at the next
/// array; a read into a constrained object of another length is an error (docs/decisions.md
/// "Binary files").
#[test]
fn binary_array_round_trip() {
    let path = scratch("binary_array.bin");
    let _ = std::fs::remove_file(&path);
    let mut pb = ProgramBuilder::new();
    let int = integer(&mut pb);
    let arr = pb.ty(Ty::Array {
        elem: int,
        elem_desc: None,
        dims: 1,
    });
    let file = pb.ty(Ty::File { elem: arr });
    let (a3, a2) = (pb.konst(ints(&[7, 8, 9])), pb.konst(ints(&[5, 6])));
    let open = |fb: &mut FuncBuilder<'_>, f: u32, kind: i64| {
        stmt(fb, StmtKind::Call);
        let (fa, name, k) = (
            fb.addr(AddrSpace::Frame, f),
            fat_str(fb, path.as_bytes()),
            fb.int(kind),
        );
        native(
            fb,
            "yavs.implicit.file_open.arr",
            Some(file),
            &[fa, name, k],
            false,
        );
    };
    let f = pb.declare("f", FuncKind::Function, None);
    {
        let mut fb = pb.define(f);
        let (cell, buf, len) = (fb.slot(4, 4), fb.slot(16, 8), fb.slot(8, 8));
        open(&mut fb, cell, 1);
        for (c, n) in [(a3, 2), (a2, 1)] {
            stmt(&mut fb, StmtKind::Call);
            let (fa, data) = (
                fb.addr(AddrSpace::Frame, cell),
                fb.addr(AddrSpace::Const(c), 0),
            );
            let v = fat_of(&mut fb, data, 0, n, false);
            native(
                &mut fb,
                "yavs.implicit.write.arr",
                Some(file),
                &[fa, v],
                false,
            );
        }
        stmt(&mut fb, StmtKind::Call);
        let fa = fb.addr(AddrSpace::Frame, cell);
        native(
            &mut fb,
            "yavs.implicit.file_close.arr",
            Some(file),
            &[fa],
            false,
        );
        open(&mut fb, cell, 0);
        let mut acc = fb.int(0);
        for _ in 0..2 {
            stmt(&mut fb, StmtKind::Call);
            let (fa, ba, la) = (
                fb.addr(AddrSpace::Frame, cell),
                fb.addr(AddrSpace::Frame, buf),
                fb.addr(AddrSpace::Frame, len),
            );
            let v = fat_of(&mut fb, ba, 0, 1, false);
            native(
                &mut fb,
                "yavs.implicit.read.arr",
                Some(file),
                &[fa, v, la],
                false,
            );
            // The 3-element array is cut to 2; the 2-element array fits.
            for r in [
                load(&mut fb, AddrSpace::Frame, len, Width::B8),
                load(&mut fb, AddrSpace::Frame, buf, Width::B4),
                load(&mut fb, AddrSpace::Frame, buf + 4, Width::B4),
            ] {
                let ten = fb.int(10);
                let s = bin(&mut fb, IntOp::Mul, acc, ten);
                acc = bin(&mut fb, IntOp::Add, s, r);
            }
        }
        stmt(&mut fb, StmtKind::Call);
        let fa = fb.addr(AddrSpace::Frame, cell);
        let eof = native(
            &mut fb,
            "yavs.implicit.endfile.arr",
            Some(file),
            &[fa],
            true,
        )
        .unwrap();
        let k = fb.int(1_000_000_000);
        let eof = bin(&mut fb, IntOp::Mul, eof, k);
        let r = bin(&mut fb, IntOp::Add, acc, eof);
        fb.emit(Inst::Ret { v: Some(r) });
        fb.finish();
    }
    // Reads the first record (3 elements) into a constrained object of 2.
    let g = pb.declare("g", FuncKind::Procedure, None);
    {
        let mut fb = pb.define(g);
        let (cell, buf) = (fb.slot(4, 4), fb.slot(16, 8));
        open(&mut fb, cell, 0);
        stmt(&mut fb, StmtKind::Call);
        let (fa, ba) = (
            fb.addr(AddrSpace::Frame, cell),
            fb.addr(AddrSpace::Frame, buf),
        );
        let v = fat_of(&mut fb, ba, 0, 1, false);
        native(
            &mut fb,
            "yavs.implicit.read.arr",
            Some(file),
            &[fa, v],
            false,
        );
        fb.emit(Inst::Ret { v: None });
        fb.finish();
    }
    let prog = Arc::new(pb.finish().unwrap());
    let rt = Runtime::new(prog, Bindings::default()).unwrap();
    let inst = Arc::new(InstData::default());
    let r = yavs_interp::call(rt.clone(), f, inst.clone(), &[]).unwrap();
    // len 2, 7, 8, then len 2, 5, 6; endfile true.
    assert_eq!(r, Some(1_000_000_000 + 278256));
    let e = yavs_interp::call(rt, g, inst, &[]).unwrap_err();
    let ErrorKind::Runtime(m) = &e.kind else {
        panic!("{e}")
    };
    assert!(m.contains("3 elements") && m.contains("2 elements"), "{m}");
}

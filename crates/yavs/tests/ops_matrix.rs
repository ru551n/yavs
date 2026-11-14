//! The operator matrix: every pair of an implicit operator family (`yavs_sem::implicit::Family`)
//! and an operand type class (`TypeClass`) is decided in `cell`, a match over the pair with no
//! wildcard, so a new family or a new class does not compile until each of its pairs is decided:
//! rows that run, `NotApplicable` (no such predefined operation), `Pending` (not guarded yet, with
//! the reason) or `Later` (a planned step covers it; `nothing_left_for_a_landed_step` fails once
//! that step is in `LANDED`). Operations no family names (`case?`, conversions) are in
//! `constructs`.
//!
//! A row is a design that runs the row's operations on operands of `W` elements, `R` times
//! (`-gR`), checking every result against a plain element loop run before the timed loop; `W`
//! is a generic (`-gW`, a length known at run time) and, for array rows, also a locally static
//! constant. The cost of one operation is the growth of the work counters from `R = 1` to
//! `R = 3`, halved and divided by the operations of an iteration. With
//! `YAVS_COUNTERS=1 --tier=interp --threads 1`:
//!
//! - a `Word` row (cost per 64-bit word or less) adds at most `WORD_SLOPE` interpreter `ops` per
//!   operation from W=64 to W=2048 (a per-element IR loop adds about 6 ops per element, 11.9 k);
//! - no `ieee` native of a `Word` row runs its general path (`ieee_general.<key>`) at W of 64 or
//!   less, nor at 2048 unless the row says why (`general_ok`), and no `yavs.implicit` native
//!   decodes an element (`implicit_elems`, counted in every build);
//! - siblings (`Row::group`: `<`/`<=`/`>`/`>=`, the array classes of one operator, ...) have
//!   slopes within 4x of each other (slopes below `WORD_SLOPE` count as `WORD_SLOPE`), unless the
//!   row names the sibling in `asym_ok` with a reason: the check that catches an operator whose
//!   sibling has a fast path it lacks;
//! - under `--features clift` (compiled before the run) no operation of any row is a host step of
//!   compiled code (`cb_step`, `cb_xstep`), and a `Word` row makes no native callback
//!   (`cb_native`) at W of 64 or less and at most one per operation above (a `native` row, an
//!   operation compiled code always calls: at most one per operation at every width).
//!
//! Results are checked in every build; `ops` and `cb_*` are counted in debug builds and with
//! `op-census` only. A changed bound is edited in the same commit with the reason (root `AGENTS.md`
//! section 4, "Work-counter tests"). Print the numbers with `YAVS_PERF_PRINT=1 -- --nocapture`.

mod common;
use common::tempdir;

use std::collections::BTreeMap;
use std::path::Path;
use std::process::Command;
use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};
use yavs_sem::implicit::Family;

/// `ops` per operation a `Word` row may add from W=64 to W=2048.
const WORD_SLOPE: u64 = 64;

/// Widths every row runs at: the slope's two, and 1 (a null prefix operand) for the results.
const WIDTHS: [u32; 3] = [1, 64, 2048];

/// The steps of the operator plan that have landed: 1 array ordering, 2 compiled code without
/// host steps for `**` and `time * real`, 3 the `yavs.implicit` array natives on planes, 4 the
/// missing natives; 5 (not started) the fixed costs of the word-wise operations (a temporary and
/// a copy per operation, a native callback per call from compiled code) and the general paths.
const LANDED: u8 = 4;

/// Designs run at once (each a `yavs` process).
const JOBS: usize = 4;

#[derive(Clone, Copy, Debug, PartialEq)]
enum TypeClass {
    Bit,
    Boolean,
    StdULogic,
    Int,
    Real,
    Phys,
    /// Enumerations other than BIT, BOOLEAN, CHARACTER and STD_ULOGIC.
    Enum,
    Char,
    Access,
    File,
    /// Arrays of STD_ULOGIC (bit planes).
    LogicArray,
    BitArray,
    BooleanArray,
    /// `string` and arrays of enumerations: one byte per element.
    ByteArray,
    /// Arrays of integers within 32 bits (4 bytes per element) and wider (8 bytes).
    IntArray,
    WideIntArray,
    RealArray,
    ArrayOfArrays,
    Record,
}

const ALL_CLASSES: &[TypeClass] = {
    use TypeClass::*;
    &[
        Bit,
        Boolean,
        StdULogic,
        Int,
        Real,
        Phys,
        Enum,
        Char,
        Access,
        File,
        LogicArray,
        BitArray,
        BooleanArray,
        ByteArray,
        IntArray,
        WideIntArray,
        RealArray,
        ArrayOfArrays,
        Record,
    ]
};

#[derive(Clone, Copy, Debug, PartialEq)]
enum Cost {
    /// Cost per 64-bit word or less.
    Word,
    /// No growth with the width (a scalar operation).
    Fixed,
}

/// One row: an operator family on one type class.
struct Row {
    name: String,
    class: TypeClass,
    cost: Cost,
    /// Rows whose slopes must agree (within 4x).
    group: String,
    /// Siblings this row may differ from, with the reason.
    asym_ok: &'static [(&'static str, &'static str)],
    /// Why an `ieee` native of the row may run its general path at W=2048.
    general_ok: Option<&'static str>,
    /// Use clauses after `ieee.std_logic_1164.all` (another `ieee` package).
    uses: &'static str,
    /// Only with a static `W` (locally static choices of `case?`).
    static_only: bool,
    /// The operation is a native call in compiled code at every width (at most one `cb_native` per
    /// operation, also at 64 elements or less).
    native: bool,
    /// A host step of compiled code the row is known to make, and the plan step that removes it
    /// (`nothing_left_for_a_landed_step` fails once that step has landed).
    step_ok: Option<(u8, &'static str)>,
    /// Declarations in a package (types) and in the process (variables, the reference), the
    /// statements before the timed loop, and the checks of odd and even iterations.
    decls: &'static str,
    vars: String,
    setup: String,
    ops1: String,
    ops0: String,
    /// The operations in one iteration.
    n_ops: u64,
}

/// What the matrix says about one pair or construct.
enum Item {
    Row(Box<Row>),
    /// No such predefined operation.
    NotApplicable,
    /// Not guarded yet, and why.
    Pending(&'static str),
    /// A step of the plan covers it: guarded by rows once that step lands.
    Later(u8, &'static str),
}

macro_rules! scalars {
    () => {
        TypeClass::Bit
            | TypeClass::Boolean
            | TypeClass::StdULogic
            | TypeClass::Int
            | TypeClass::Real
            | TypeClass::Phys
            | TypeClass::Enum
            | TypeClass::Char
    };
}

/// One-dimensional arrays of discrete elements (those `<` orders, LRM 9.2.3).
macro_rules! discrete_arrays {
    () => {
        TypeClass::LogicArray
            | TypeClass::BitArray
            | TypeClass::BooleanArray
            | TypeClass::ByteArray
            | TypeClass::IntArray
            | TypeClass::WideIntArray
    };
}

const SCALAR_INLINE: &str = "a scalar operation in line; no row yet";

/// The decision for the family `f` on operands of the class `c`.
fn cell(f: Family, c: TypeClass) -> Vec<Item> {
    use Family::*;
    use Item::{Later, NotApplicable as Na, Pending};
    use TypeClass::*;
    let rows = |r: Vec<Row>| {
        r.into_iter()
            .map(|r| Item::Row(Box::new(r)))
            .collect::<Vec<_>>()
    };
    match (f, c) {
        // Relations (LRM 9.2.3).
        (Eq | Neq, discrete_arrays!()) => rows(eq_rows(f, c)),
        (Eq | Neq, RealArray) => vec![Pending(
            "an element loop (-0.0 = 0.0 needs a value compare, #1144); no row yet",
        )],
        (Eq | Neq, ArrayOfArrays | Record) => vec![Pending(
            "`comp_eq`: one `LEq` over logic elements, else per field; no row yet",
        )],
        (Eq | Neq, scalars!() | Access) => vec![Pending(SCALAR_INLINE)],
        (Eq | Neq, File) => vec![Na],
        (Lt | Le | Gt | Ge, discrete_arrays!()) => rows(order_rows(f, c)),
        (Lt | Le | Gt | Ge, scalars!()) => vec![Pending(SCALAR_INLINE)],
        (Lt | Le | Gt | Ge, RealArray | ArrayOfArrays | Record | Access | File) => vec![Na],
        (Minimum | Maximum, discrete_arrays!()) => {
            let mut v = rows(minmax_rows(f, c));
            v.push(Pending(
                "the one-operand reduction form looks at every element by nature",
            ));
            v
        }
        (Minimum | Maximum, RealArray) => vec![Pending(
            "only the one-operand reduction form, which looks at every element by nature",
        )],
        (Minimum | Maximum, scalars!()) => vec![Pending("a scalar native; no row yet")],
        (Minimum | Maximum, ArrayOfArrays | Record | Access | File) => vec![Na],
        // Strings (LRM 5.7).
        (ToString, scalars!()) => vec![Pending("the std image natives; no row yet")],
        (ToString, LogicArray | BitArray) => rows(tostr_rows(c)),
        (ToString, ByteArray) => vec![Pending(
            "arrays of a character-literal enumeration other than BIT; no row yet",
        )],
        (
            ToString,
            BooleanArray | IntArray | WideIntArray | RealArray | ArrayOfArrays | Record | Access
            | File,
        ) => vec![Na],
        (ToHString | ToOString, BitArray) => rows(radix_rows(f)),
        (ToHString | ToOString, LogicArray) => {
            vec![Pending("the ieee `Str` native, guarded by `ieee_paths`")]
        }
        (
            ToHString | ToOString,
            scalars!()
            | Access
            | File
            | BooleanArray
            | ByteArray
            | IntArray
            | WideIntArray
            | RealArray
            | ArrayOfArrays
            | Record,
        ) => vec![Na],
        // Arithmetic (LRM 9.2.6 to 9.2.8).
        (Add | Sub | Neg | Pos | Abs | Mul | Div, Int | Real) => vec![Pending(SCALAR_INLINE)],
        (Add | Sub | Neg | Pos | Abs, Phys) => vec![Pending(SCALAR_INLINE)],
        (Mul, Phys) => rows(phys_rows(f).into_iter().chain(call_rows()).collect()),
        (Div, Phys) => rows(phys_rows(f)),
        (Rem | Mod, Int | Phys) => vec![Pending(SCALAR_INLINE)],
        (Rem | Mod, Real) => vec![Na],
        (Exp, Int | Real) => rows(exp_rows(c)),
        (Exp, Phys) => vec![Na],
        (
            Add | Sub | Mul | Div | Neg | Pos | Abs | Exp | Rem | Mod,
            Bit
            | Boolean
            | StdULogic
            | Enum
            | Char
            | Access
            | File
            | discrete_arrays!()
            | RealArray
            | ArrayOfArrays
            | Record,
        ) => vec![Na],
        // Logical operators (LRM 9.2.2).
        (And | Or | Nand | Nor | Xor | Xnor | Not, Bit | Boolean) => {
            vec![Pending(
                "in line (`Bool`, short-circuit branches); no row yet",
            )]
        }
        (And | Or | Nand | Nor | Xor | Xnor | Not, StdULogic) => {
            vec![Pending("the ieee `Gate` native, guarded by `ieee_paths`")]
        }
        (And | Or | Nand | Nor | Xor | Xnor, BitArray | BooleanArray) => rows(
            logic_rows(f, c)
                .into_iter()
                .chain(logic_form_rows(f, c))
                .collect(),
        ),
        (And | Or | Nand | Nor | Xor | Xnor, LogicArray) => {
            let mut v = rows(logic_rows(f, c));
            v.extend(rows(slv_scalar_rows(f)));
            v.push(Pending(
                "the reductions: the ieee `Reduce` native, guarded by `ieee_paths`",
            ));
            v
        }
        (Not, BitArray | BooleanArray | LogicArray) => rows(logic_rows(f, c)),
        (
            And | Or | Nand | Nor | Xor | Xnor | Not,
            Int | Real | Phys | Enum | Char | Access | File | ByteArray | IntArray | WideIntArray
            | RealArray | ArrayOfArrays | Record,
        ) => vec![Na],
        // Matching operators and the condition operator (LRM 9.2.3, 9.2.9).
        (MEq | MNeq, Bit) => vec![Pending("in line `Cmp`; no row yet")],
        (MEq | MNeq, StdULogic) => {
            vec![Pending(
                "the ieee `MatchEq` native, guarded by `ieee_paths`",
            )]
        }
        (MEq | MNeq, BitArray) => rows(match_rows(f, c)),
        (MEq | MNeq, LogicArray) => vec![
            Pending("std_ulogic_vector: the ieee `MatchEq` native, guarded by `ieee_paths`"),
            Later(
                5,
                "numeric_std `?=` above 1024 elements: the general path on the heap",
            ),
        ],
        (MLt | MLe | MGt | MGe, Bit | StdULogic) => {
            vec![Pending("a scalar native (a fixed cost); no row yet")]
        }
        (MLt | MLe | MGt | MGe, LogicArray) => vec![Later(
            5,
            "numeric_std `?<` and the like above 1024 elements: the general path on the heap",
        )],
        (MEq | MNeq, Boolean | Int | Real | Phys | Enum | Char | Access | File)
        | (MLt | MLe | MGt | MGe, Boolean | Int | Real | Phys | Enum | Char | Access | File)
        | (
            MEq | MNeq,
            BooleanArray | ByteArray | IntArray | WideIntArray | RealArray | ArrayOfArrays | Record,
        )
        | (
            MLt | MLe | MGt | MGe,
            BitArray | BooleanArray | ByteArray | IntArray | WideIntArray | RealArray
            | ArrayOfArrays | Record,
        ) => vec![Na],
        (Cond, Bit) => vec![Pending("in line; no row yet")],
        (Cond, StdULogic) => vec![Pending("the ieee `Cond` native, guarded by `ieee_paths`")],
        (
            Cond,
            Boolean
            | Int
            | Real
            | Phys
            | Enum
            | Char
            | Access
            | File
            | discrete_arrays!()
            | RealArray
            | ArrayOfArrays
            | Record,
        ) => vec![Na],
        // Concatenation (LRM 9.2.5).
        (Cat, LogicArray | BitArray | BooleanArray | ByteArray) => vec![Later(
            5,
            "a temporary and a copy into the target per operation (a fixed cost of 1-2 k \
             instructions)",
        )],
        (Cat, IntArray | WideIntArray | RealArray | ArrayOfArrays) => {
            vec![Pending("one `Cat` of bytes; no row yet")]
        }
        (Cat, scalars!() | Access | File | Record) => vec![Na],
        // Shifts and rotations (LRM 9.2.4).
        (Sll | Srl | Sla | Sra | Rol | Ror, BitArray | BooleanArray) => rows(shift_rows(f, c)),
        (Sll | Srl | Rol | Ror, LogicArray) => vec![Pending(
            "the std_logic_1164 bodies: slice copies, word-wise; no row yet",
        )],
        (Sla | Sra, LogicArray) => vec![Na],
        (
            Sll | Srl | Sla | Sra | Rol | Ror,
            scalars!()
            | Access
            | File
            | ByteArray
            | IntArray
            | WideIntArray
            | RealArray
            | ArrayOfArrays
            | Record,
        ) => vec![Na],
        // Storage, files and edges (LRM 5.4.3, 5.5.2, 16.3).
        (Deallocate, Access) => vec![Pending("no width to guard")],
        (
            Deallocate,
            scalars!() | File | discrete_arrays!() | RealArray | ArrayOfArrays | Record,
        ) => vec![Na],
        (
            FileOpen | FileClose | Read | Write | Endfile | Flush | FileState | FileMode
            | FilePosition | FileSize | FileSeek | FileTruncate | FileRewind | FileCanseek,
            File,
        ) => vec![Pending("file operations: bound by the file system")],
        (
            FileOpen | FileClose | Read | Write | Endfile | Flush | FileState | FileMode
            | FilePosition | FileSize | FileSeek | FileTruncate | FileRewind | FileCanseek,
            scalars!() | Access | discrete_arrays!() | RealArray | ArrayOfArrays | Record,
        ) => vec![Na],
        (RisingEdge | FallingEdge, Bit | Boolean | StdULogic) => vec![Pending(
            "the clock-edge forms of compiled code, guarded by `clift_parity`",
        )],
        (
            RisingEdge | FallingEdge,
            Int
            | Real
            | Phys
            | Enum
            | Char
            | Access
            | File
            | discrete_arrays!()
            | RealArray
            | ArrayOfArrays
            | Record,
        ) => vec![Na],
    }
}

/// Operations no implicit family names: `ieee` subprograms and statements.
fn constructs() -> Vec<Item> {
    use Item::{Later, Pending};
    let mut v: Vec<Item> = [tosulv_row(), caseq_row()]
        .into_iter()
        .chain(nbit_rows())
        .map(|r| Item::Row(Box::new(r)))
        .collect();
    v.extend([
        Pending(
            "numeric_bit reductions: the body calls the predefined reduction of bit_vector, a \
             native on planes; no row of its own",
        ),
        Later(
            5,
            "`ieee` natives of 65 to 1024 elements from compiled code: a native callback each \
             (about 200 instructions more than the interpreter's call)",
        ),
        Pending("textio `read`/`write` of std_ulogic_vector: the bodies, per character"),
    ]);
    v
}

const ALL_FAMILIES: &[Family] = {
    use Family::*;
    &[
        Eq,
        Neq,
        Lt,
        Le,
        Gt,
        Ge,
        Minimum,
        Maximum,
        ToString,
        Add,
        Sub,
        Mul,
        Div,
        Neg,
        Pos,
        Abs,
        Exp,
        Rem,
        Mod,
        And,
        Or,
        Nand,
        Nor,
        Xor,
        Xnor,
        Not,
        MEq,
        MNeq,
        MLt,
        MLe,
        MGt,
        MGe,
        Cond,
        Cat,
        Sll,
        Srl,
        Sla,
        Sra,
        Rol,
        Ror,
        ToOString,
        ToHString,
        Deallocate,
        FileOpen,
        FileClose,
        Read,
        Write,
        Endfile,
        Flush,
        FileState,
        FileMode,
        FilePosition,
        FileSize,
        FileSeek,
        FileTruncate,
        FileRewind,
        FileCanseek,
        RisingEdge,
        FallingEdge,
    ]
};

/// Every item: each pair of the matrix, then the constructs.
fn all_items() -> Vec<Item> {
    let mut v: Vec<Item> = ALL_FAMILIES
        .iter()
        .flat_map(|&f| ALL_CLASSES.iter().flat_map(move |&c| cell(f, c)))
        .collect();
    v.extend(constructs());
    v
}

fn all_rows() -> Vec<Row> {
    all_items()
        .into_iter()
        .filter_map(|i| match i {
            Item::Row(r) => Some(*r),
            _ => None,
        })
        .collect()
}

/// A reference `lex` (-1, 0, 1) for an array type `T` of a scalar element type with `<`.
macro_rules! lex {
    ($t:literal) => {
        concat!(
            "function lex(x, y : ",
            $t,
            ") return integer is
    variable i, j : integer;
  begin
    for k in 0 to minimum(x'length, y'length) - 1 loop
      if x'ascending then i := x'left + k; else i := x'left - k; end if;
      if y'ascending then j := y'left + k; else j := y'left - k; end if;
      if x(i) /= y(j) then
        if x(i) < y(j) then return -1; else return 1; end if;
      end if;
    end loop;
    if x'length < y'length then return -1; elsif x'length > y'length then return 1; end if;
    return 0;
  end function;"
        )
    };
}

/// An array operand class: its type, ranges of `W` and `W - 1` elements, element `i` of `a`, the
/// changed rightmost element `{x}` of `b1`, package declarations and the reference `lex`.
struct Arr {
    class: TypeClass,
    ty: &'static str,
    full: &'static str,
    short: &'static str,
    elem: &'static str,
    toggle: &'static str,
    decls: &'static str,
    lex: &'static str,
}

/// The operand class `c` (one of the discrete arrays).
fn array(c: TypeClass) -> Arr {
    use TypeClass::*;
    match c {
        LogicArray => Arr {
            class: LogicArray,
            ty: "std_ulogic_vector({r})",
            full: "W - 1 downto 0",
            short: "W - 1 downto 1",
            elem: "std_ulogic'val(2 + i mod 7)",
            toggle: "'X' when {x} /= 'X' else 'H'",
            decls: "",
            lex: lex!("std_ulogic_vector"),
        },
        BitArray => Arr {
            class: BitArray,
            ty: "bit_vector({r})",
            full: "W - 1 downto 0",
            short: "W - 1 downto 1",
            elem: "'1' when i mod 3 = 0 else '0'",
            toggle: "not {x}",
            decls: "",
            lex: lex!("bit_vector"),
        },
        BooleanArray => Arr {
            class: BooleanArray,
            ty: "boolean_vector({r})",
            full: "0 to W - 1",
            short: "0 to W - 2",
            elem: "i mod 5 = 1",
            toggle: "not {x}",
            decls: "",
            lex: lex!("boolean_vector"),
        },
        ByteArray => Arr {
            class: ByteArray,
            ty: "string({r})",
            full: "1 to W",
            short: "1 to W - 1",
            elem: "character'val(120 + i mod 20)",
            toggle: "character'val(255) when {x} /= character'val(255) else nul",
            decls: "",
            lex: lex!("string"),
        },
        IntArray => Arr {
            class: IntArray,
            ty: "integer_vector({r})",
            full: "0 to W - 1",
            short: "0 to W - 2",
            elem: "i * 7919 - 4000",
            toggle: "-{x} - 1",
            decls: "",
            lex: lex!("integer_vector"),
        },
        WideIntArray => Arr {
            class: WideIntArray,
            ty: "big_vector({r})",
            full: "W - 1 downto 0",
            short: "W - 1 downto 1",
            elem: "big(i mod 1000) * 1073741827 - 536870912",
            toggle: "-{x} - 1",
            decls: "type big is range -1099511627776 to 1099511627776;
  type big_vector is array (natural range <>) of big;",
            lex: lex!("big_vector"),
        },
        _ => unreachable!("{c:?} is no discrete array class"),
    }
}

/// The row of family `name` on the array class `c`: the pair `a`, `{b}` of equal lengths (equal
/// on even iterations, the rightmost element changed on odd ones; `{e}` the reference order of
/// `a` and `{b}`) and the prefix `c` of `a` (`W - 1` elements, null at W=1).
fn array_row(name: &str, group: &str, c: TypeClass, ops: &str, n_ops: u64) -> Row {
    let t = array(c);
    let ty = |r: &str| t.ty.replace("{r}", r);
    let toggle = t.toggle.replace("{x}", "b1(b1'right)");
    Row {
        name: format!("{name}_{:?}", t.class),
        class: t.class,
        cost: Cost::Word,
        group: group.to_string(),
        asym_ok: &[],
        general_ok: None,
        native: false,
        uses: "",
        static_only: false,
        step_ok: None,
        decls: t.decls,
        vars: format!(
            "variable a, b0, b1 : {};\n    variable c : {};\n    variable e1 : integer;\n    {}",
            ty(t.full),
            ty(t.short),
            t.lex
        ),
        setup: format!(
            "for i in a'range loop a(i) := {}; end loop;
    b0 := a;
    b1 := a;
    b1(b1'right) := {toggle};
    e1 := lex(a, b1);
    for i in c'range loop c(i) := a(i); end loop;",
            t.elem
        ),
        ops1: ops.replace("{b}", "b1").replace("{e}", "e1"),
        ops0: ops.replace("{b}", "b0").replace("{e}", "0"),
        n_ops,
    }
}

/// `<`, `<=`, `>`, `>=` of one-dimensional discrete arrays (LRM 9.2.3), both ways round and
/// against the prefix.
fn order_rows(f: Family, c: TypeClass) -> Vec<Row> {
    let (name, ops) = match f {
        Family::Lt => (
            "lt",
            "check(a < {b}, {e} < 0); check({b} < a, {e} > 0); check(c < {b}, true); check({b} < c, false);",
        ),
        Family::Le => (
            "le",
            "check(a <= {b}, {e} <= 0); check({b} <= a, {e} >= 0); check(c <= {b}, true); check({b} <= c, false);",
        ),
        Family::Gt => (
            "gt",
            "check(a > {b}, {e} > 0); check({b} > a, {e} < 0); check(c > {b}, false); check({b} > c, true);",
        ),
        _ => (
            "ge",
            "check(a >= {b}, {e} >= 0); check({b} >= a, {e} <= 0); check(c >= {b}, false); check({b} >= c, true);",
        ),
    };
    vec![array_row(name, "order", c, ops, 4)]
}

/// `=` and `/=` of one-dimensional arrays: the pair, and the prefix (other lengths: unequal).
fn eq_rows(f: Family, c: TypeClass) -> Vec<Row> {
    let (name, ops) = match f {
        Family::Eq => (
            "eq",
            "check(a = {b}, {e} = 0); check({b} = a, {e} = 0); check(c = {b}, false); check({b} = c, false);",
        ),
        _ => (
            "neq",
            "check(a /= {b}, {e} /= 0); check({b} /= a, {e} /= 0); check(c /= {b}, true); check({b} /= c, true);",
        ),
    };
    vec![array_row(name, "eq", c, ops, 4)]
}

/// The binary logical operators and `not` of arrays (`a op b`, element by element, LRM 9.2.2):
/// both ways round, against results made by an element loop. On std_ulogic_vector these are the
/// std_logic_1164 natives.
fn logic_rows(f: Family, c: TypeClass) -> Vec<Row> {
    let op = f.as_str();
    let mut r = array_row(op, op, c, "", 2);
    if c == TypeClass::LogicArray {
        // '0' and '1' only: a metavalue sends the std_logic_1164 natives to their general path.
        let t = array(c);
        r.setup = r
            .setup
            .replace(t.elem, "'1' when i mod 3 = 0 else '0'")
            .replace(&t.toggle.replace("{x}", "b1(b1'right)"), "not b1(b1'right)");
    }
    // `not` of the second operand: `r0` is `not b0`, `r1` is `not b1`.
    let elem = |x: &str, y: &str| match f {
        Family::Not => format!("not {y}(i)"),
        _ => format!("{x}(i) {op} {y}(i)"),
    };
    r.vars += &format!(
        "\n    variable r0, r1 : {};",
        array(c).ty.replace("{r}", array(c).full)
    );
    r.setup += &format!(
        "\n    for i in a'range loop r0(i) := {}; r1(i) := {}; end loop;",
        elem("a", "b0"),
        elem("a", "b1")
    );
    let (ops1, ops0) = match f {
        Family::Not => (
            "check((not b1) = r1, true); check((not a) = r0, true);".to_string(),
            "check((not b0) = r0, true); check((not a) = r0, true);".to_string(),
        ),
        _ => (
            format!("check((a {op} b1) = r1, true); check((b1 {op} a) = r1, true);"),
            format!("check((a {op} b0) = r0, true); check((b0 {op} a) = r0, true);"),
        ),
    };
    (r.ops1, r.ops0) = (ops1, ops0);
    if c == TypeClass::LogicArray {
        r.general_ok = Some(
            "above 1024 elements the std_logic_1164 natives take their general path, word by \
             word on a `LogicVec`",
        );
    }
    vec![r]
}

/// The element type and the literals of '0' and '1' of a BIT or BOOLEAN array class.
fn bit_class(c: TypeClass) -> (&'static str, &'static str, &'static str) {
    match c {
        TypeClass::BitArray => ("bit", "'0'", "'1'"),
        _ => ("boolean", "false", "true"),
    }
}

/// `array_row` for an operation that is a `yavs.implicit` native (a call in compiled code).
fn native_row(name: &str, group: &str, c: TypeClass) -> (Row, Arr) {
    let mut r = array_row(name, group, c, "", 2);
    r.native = true;
    (r, array(c))
}

/// The type of `t` over `W` elements.
fn full(t: &Arr) -> String {
    t.ty.replace("{r}", t.full)
}

/// Shifts and rotations of BIT and BOOLEAN arrays (LRM 9.2.4): by a static 3 and by a count from
/// a variable (`W / 3 + 1`, the whole length at W=1), of both signs, against an element loop.
fn shift_rows(f: Family, c: TypeClass) -> Vec<Row> {
    let op = f.as_str();
    let code = match f {
        Family::Sll => 0,
        Family::Srl => 1,
        Family::Sla => 2,
        Family::Sra => 3,
        Family::Rol => 4,
        _ => 5,
    };
    let (_, zero, _) = bit_class(c);
    let (mut r, t) = native_row(op, op, c);
    let ty = t.ty.replace("({r})", "");
    r.vars += &format!(
        "
    variable kv : integer;
    variable e3, e4, e5, e6 : {ty}(0 to W - 1);
    function sh(x : {ty}; op : natural; k : integer) return {ty} is
      constant n : natural := x'length;
      alias v : {ty}(0 to n - 1) is x;
      variable r : {ty}(0 to n - 1);
      variable kk : integer := k;
      variable o : natural := op;
      variable j : integer;
    begin
      if kk < 0 then kk := -kk; o := o + 1 - 2 * (o mod 2); end if;
      for i in 0 to n - 1 loop
        case o is
          when 0 => j := i + kk; if j < n then r(i) := v(j); else r(i) := {zero}; end if;
          when 1 => j := i - kk; if j >= 0 then r(i) := v(j); else r(i) := {zero}; end if;
          when 2 => j := i + kk; if j < n then r(i) := v(j); else r(i) := v(n - 1); end if;
          when 3 => j := i - kk; if j >= 0 then r(i) := v(j); else r(i) := v(0); end if;
          when 4 => r(i) := v((i + kk) mod n);
          when others => r(i) := v((i - kk) mod n);
        end case;
      end loop;
      return r;
    end function;"
    );
    r.setup += &format!(
        "
    kv := W / 3 + 1;
    e3 := sh(a, {code}, 3); e4 := sh(b1, {code}, kv);
    e5 := sh(a, {code}, kv); e6 := sh(b1, {code}, -kv);"
    );
    r.ops1 = format!("check((a {op} 3) = e3, true); check((b1 {op} kv) = e4, true);");
    r.ops0 = format!("check((a {op} kv) = e5, true); check((b1 {op} (-kv)) = e6, true);");
    vec![r]
}

/// The reduction (`and v`, LRM 9.2.2: the base operator from its unit, inverted for `nand`,
/// `nor`, `xnor`) and the array-scalar forms of a logical operator on BIT and BOOLEAN arrays,
/// against element loops.
fn logic_form_rows(f: Family, c: TypeClass) -> Vec<Row> {
    let op = f.as_str();
    let (el, zero, one) = bit_class(c);
    let (base, unit, inv) = match f {
        Family::And => ("and", one, ""),
        Family::Nand => ("and", one, " ra := not ra; rb := not rb;"),
        Family::Or => ("or", zero, ""),
        Family::Nor => ("or", zero, " ra := not ra; rb := not rb;"),
        Family::Xor => ("xor", zero, ""),
        _ => ("xor", zero, " ra := not ra; rb := not rb;"),
    };
    let red_name = format!("{op}_reduce");
    let (mut red, _) = native_row(&red_name, &red_name, c);
    red.vars += &format!("\n    variable ra, rb : {el};");
    red.setup += &format!(
        "
    ra := {unit}; rb := {unit};
    for i in a'range loop ra := ra {base} a(i); rb := rb {base} b1(i); end loop;{inv}"
    );
    red.ops1 = format!("check(({op} b1) = rb, true); check(({op} a) = ra, true);");
    red.ops0 = format!("check(({op} b0) = ra, true); check(({op} a) = ra, true);");
    let sc_name = format!("{op}_scalar");
    let (mut sc, t) = native_row(&sc_name, &sc_name, c);
    sc.vars += &format!(
        "\n    variable s0, s1 : {el};\n    variable q0, q1, q2, q3 : {};",
        full(&t)
    );
    sc.setup += &format!(
        "
    s0 := {zero}; s1 := {one};
    for i in a'range loop
      q0(i) := a(i) {op} s1; q1(i) := s0 {op} a(i); q2(i) := a(i) {op} s0; q3(i) := s1 {op} a(i);
    end loop;"
    );
    sc.ops1 = format!("check((a {op} s1) = q0, true); check((s0 {op} a) = q1, true);");
    sc.ops0 = format!("check((a {op} s0) = q2, true); check((s1 {op} a) = q3, true);");
    vec![red, sc]
}

/// `?=` and `?/=` of bit_vector (LRM 9.2.3: BIT has no metavalue, so `=` decides).
fn match_rows(f: Family, c: TypeClass) -> Vec<Row> {
    let (op, ne, eq) = match f {
        Family::MEq => ("?=", "'0'", "'1'"),
        _ => ("?/=", "'1'", "'0'"),
    };
    let (mut r, _) = native_row(f.as_str(), "match", c);
    r.ops1 = format!("check((a {op} b1) = {ne}, true); check((b1 {op} a) = {ne}, true);");
    r.ops0 = format!("check((a {op} b0) = {eq}, true); check((b0 {op} a) = {eq}, true);");
    vec![r]
}

/// The predefined `minimum` and `maximum` of two one-dimensional discrete arrays (LRM 5.2.6:
/// `MINIMUM(L, R)` is `L` when `L < R`, else `R`): the pair and the prefix.
fn minmax_rows(f: Family, c: TypeClass) -> Vec<Row> {
    let name = f.as_str();
    let (mut r, t) = native_row(name, "minmax", c);
    r.vars += &format!("\n    variable m1 : {};", full(&t));
    let (pick, ops1, ops0) = match f {
        Family::Minimum => (
            "e1 < 0",
            "check(minimum(a, b1) = m1, true); check(minimum(c, b1) = c, true);",
            "check(minimum(a, b0) = a, true); check(minimum(b0, c) = c, true);",
        ),
        _ => (
            "e1 > 0",
            "check(maximum(a, b1) = m1, true); check(maximum(c, b1) = b1, true);",
            "check(maximum(a, b0) = a, true); check(maximum(b0, c) = b0, true);",
        ),
    };
    r.setup += &format!("\n    if {pick} then m1 := a; else m1 := b1; end if;");
    (r.ops1, r.ops0) = (ops1.to_string(), ops0.to_string());
    vec![r]
}

/// `to_hstring` and `to_ostring` of bit_vector (LRM 5.3.2.4: zero-extended on the left to whole
/// digits), against digits made by an element loop.
fn radix_rows(f: Family) -> Vec<Row> {
    let (name, w) = match f {
        Family::ToHString => ("to_hstring", 4),
        _ => ("to_ostring", 3),
    };
    let (mut r, _) = native_row(name, "radix", TypeClass::BitArray);
    r.vars += &format!(
        "
    variable h0, h1 : string(1 to (W + {}) / {w});
    function rx(x : bit_vector; w : positive) return string is
      constant nd : natural := (x'length + w - 1) / w;
      variable v : bit_vector(nd * w - 1 downto 0) := (others => '0');
      variable s : string(1 to nd);
      variable d : natural;
      constant digits : string(1 to 16) := \"0123456789ABCDEF\";
    begin
      v(x'length - 1 downto 0) := x;
      for k in 0 to nd - 1 loop
        d := 0;
        for j in w - 1 downto 0 loop d := d * 2 + bit'pos(v((nd - 1 - k) * w + j)); end loop;
        s(k + 1) := digits(d + 1);
      end loop;
      return s;
    end function;",
        w - 1
    );
    r.setup += &format!("\n    h0 := rx(a, {w}); h1 := rx(b1, {w});");
    r.ops1 = format!("check({name}(b1) = h1, true); check({name}(a) = h0, true);");
    r.ops0 = format!("check({name}(b0) = h0, true); check({name}(a) = h0, true);");
    vec![r]
}

/// `to_string` of a std_ulogic_vector (every value, and '0'/'1' only) and of a bit_vector, against
/// strings spelled by an element loop from each element's `'image`.
fn tostr_rows(c: TypeClass) -> Vec<Row> {
    let one = |name: &str, binary: bool| {
        let (mut r, t) = native_row(name, "to_string", c);
        if binary {
            r.setup = r
                .setup
                .replace(t.elem, "'1' when i mod 3 = 0 else '0'")
                .replace(&t.toggle.replace("{x}", "b1(b1'right)"), "not b1(b1'right)");
        }
        let et = if c == TypeClass::BitArray {
            "bit"
        } else {
            "std_ulogic"
        };
        r.vars += "\n    variable s0, s1 : string(1 to W);\n    variable p : positive;";
        r.setup += &format!(
            "\n    p := 1;
    for i in a'range loop s0(p) := {et}'image(a(i))(2); s1(p) := {et}'image(b1(i))(2); p := p + 1; end loop;"
        );
        r.ops1 = "check(to_string(b1) = s1, true); check(to_string(a) = s0, true);".into();
        r.ops0 = "check(to_string(b0) = s0, true); check(to_string(a) = s0, true);".into();
        r
    };
    let mut v = vec![one("to_string", false)];
    if c == TypeClass::LogicArray {
        v.push(one("to_string_01", true));
    }
    v
}

/// The std_logic_1164 gates of a std_ulogic_vector and a std_ulogic, both sides ('0'/'1' data:
/// a metavalue sends the natives to their general path), against element loops.
fn slv_scalar_rows(f: Family) -> Vec<Row> {
    let op = f.as_str();
    let name = format!("{op}_scalar");
    let (mut r, t) = native_row(&name, &name, TypeClass::LogicArray);
    r.setup = r
        .setup
        .replace(t.elem, "'1' when i mod 3 = 0 else '0'")
        .replace(&t.toggle.replace("{x}", "b1(b1'right)"), "not b1(b1'right)");
    r.general_ok = Some(
        "above 64 elements the vector-scalar gates take their general path, word by word on a \
         `LogicVec`",
    );
    r.vars += &format!(
        "\n    variable s0, s1 : std_ulogic;\n    variable q0, q1, q2, q3 : {};",
        full(&t)
    );
    r.setup += &format!(
        "
    s0 := '0'; s1 := '1';
    for i in a'range loop
      q0(i) := a(i) {op} s1; q1(i) := s0 {op} a(i); q2(i) := a(i) {op} s0; q3(i) := s1 {op} a(i);
    end loop;"
    );
    r.ops1 = format!("check((a {op} s1) = q0, true); check((s0 {op} a) = q1, true);");
    r.ops0 = format!("check((a {op} s0) = q2, true); check((s1 {op} a) = q3, true);");
    vec![r]
}

/// `to_stdulogicvector` of a bit_vector (the same codes), against an element loop.
fn tosulv_row() -> Row {
    let (mut r, _) = native_row("to_stdulogicvector", "", TypeClass::BitArray);
    r.vars += "\n    variable u0, u1 : std_ulogic_vector(W - 1 downto 0);";
    r.setup += "
    for i in a'range loop
      u0(i) := '1' when a(i) = '1' else '0'; u1(i) := '1' when b1(i) = '1' else '0';
    end loop;";
    r.ops1 = "check(to_stdulogicvector(b1) = u1, true); check(to_stdulogicvector(a) = u0, true);"
        .to_string();
    r.ops0 = "check(to_stdulogicvector(b0) = u0, true); check(to_stdulogicvector(a) = u0, true);"
        .to_string();
    r
}

/// `case?` on a std_ulogic_vector with two locally static choices ('-' in one), against a
/// reference made of scalar `?=` (static `W` only: the choices must be locally static).
fn caseq_row() -> Row {
    let (mut r, t) = native_row("case_match", "", TypeClass::LogicArray);
    r.native = false;
    r.static_only = true;
    r.general_ok =
        Some("above 1024 elements `?=` takes its general path, word by word on a `LogicVec`");
    r.setup = r
        .setup
        .replace(t.elem, "'1' when i mod 3 = 0 else '0'")
        .replace(&t.toggle.replace("{x}", "b1(b1'right)"), "not b1(b1'right)");
    r.vars += "
    constant C0 : std_ulogic_vector(W - 1 downto 0) := (0 => '1', others => '-');
    constant C1 : std_ulogic_vector(W - 1 downto 0) := (others => '0');
    variable kc, ka, kb : integer;
    function ci(x : std_ulogic_vector) return integer is
      variable m0, m1 : boolean := true;
    begin
      for i in x'range loop
        m0 := m0 and (x(i) ?= C0(i)) = '1';
        m1 := m1 and (x(i) ?= C1(i)) = '1';
      end loop;
      if m0 then return 0; elsif m1 then return 1; end if;
      return 2;
    end function;";
    r.setup += "\n    ka := ci(a); kb := ci(b1);";
    let case = |x: &str, want: &str| {
        format!(
            "case? {x} is when C0 => kc := 0; when C1 => kc := 1; when others => kc := 2; end case?;
        check(kc = {want}, true);"
        )
    };
    r.ops1 = format!("{}\n        {}", case("b1", "kb"), case("a", "ka"));
    r.ops0 = format!("{}\n        {}", case("b0", "ka"), case("a", "ka"));
    r
}

/// numeric_bit unary `-` and `abs` of a signed and `minimum` of two unsigned, against element
/// loops and the numeric_bit `<`.
fn nbit_rows() -> Vec<Row> {
    let one = |name: &str, ty: &str, vars: &str, setup: &str, ops1: &str, ops0: &str| {
        let (mut r, _) = native_row(name, "", TypeClass::BitArray);
        r.native = false;
        r.uses = "use ieee.numeric_bit.all;";
        r.general_ok = Some(
            "above 1024 elements the numeric natives take their general path, word by word on a \
             `LogicVec`",
        );
        r.vars = r.vars.replace("bit_vector(", &format!("{ty}("));
        r.vars = r.vars.replace("x, y : bit_vector", &format!("x, y : {ty}"));
        r.vars += vars;
        r.setup += setup;
        (r.ops1, r.ops0) = (ops1.to_string(), ops0.to_string());
        r
    };
    let neg = "
    variable na, nb : signed(W - 1 downto 0);
    function ng(x : signed) return signed is
      variable r : signed(x'length - 1 downto 0);
      alias v : signed(x'length - 1 downto 0) is x;
      variable c : bit := '1';
    begin
      for i in 0 to x'length - 1 loop r(i) := not v(i) xor c; c := c and not v(i); end loop;
      return r;
    end function;";
    vec![
        one(
            "nbit_neg",
            "signed",
            neg,
            "\n    na := ng(a); nb := ng(b1);",
            "check(bit_vector(-b1) = bit_vector(nb), true); check(bit_vector(-a) = bit_vector(na), true);",
            "check(bit_vector(-b0) = bit_vector(na), true); check(bit_vector(-a) = bit_vector(na), true);",
        ),
        one(
            "nbit_abs",
            "signed",
            neg,
            "
    -- (The sign bit set: abs negates.)
    a(a'left) := '1'; b0 := a; b1(b1'left) := '1';
    if a(a'left) = '1' then na := ng(a); else na := a; end if;
    if b1(b1'left) = '1' then nb := ng(b1); else nb := b1; end if;",
            "check(bit_vector(abs b1) = bit_vector(nb), true); check(bit_vector(abs a) = bit_vector(na), true);",
            "check(bit_vector(abs b0) = bit_vector(na), true); check(bit_vector(abs a) = bit_vector(na), true);",
        ),
        one(
            "nbit_minimum",
            "unsigned",
            "\n    variable m1 : unsigned(W - 1 downto 0);",
            "\n    if a < b1 then m1 := a; else m1 := b1; end if;",
            "check(bit_vector(minimum(a, b1)) = bit_vector(m1), true); check(bit_vector(minimum(b1, a)) = bit_vector(m1), true);",
            "check(bit_vector(minimum(a, b0)) = bit_vector(a), true); check(bit_vector(minimum(b0, a)) = bit_vector(a), true);",
        ),
    ]
}

/// A scalar row: variables, their setup and the checks of odd and even iterations.
fn scalar(name: &str, class: TypeClass, vars: &str, setup: &str, ops1: &str, ops0: &str) -> Row {
    Row {
        name: name.to_string(),
        class,
        cost: Cost::Fixed,
        group: String::new(),
        asym_ok: &[],
        general_ok: None,
        native: false,
        uses: "",
        static_only: false,
        step_ok: None,
        decls: "",
        vars: vars.to_string(),
        setup: setup.to_string(),
        ops1: ops1.to_string(),
        ops0: ops0.to_string(),
        n_ops: 2,
    }
}

/// `**` of integers and of reals (an integer exponent). The operands come from
/// variables, so nothing folds.
fn exp_rows(c: TypeClass) -> Vec<Row> {
    use TypeClass::*;
    vec![match c {
        Int => scalar(
            "exp_Int",
            Int,
            "variable x, n : integer;",
            "x := W mod 7 + 3; n := 3;",
            "check(x ** n = x * x * x, true); check((-x) ** n = -(x * x * x), true);",
            "check(x ** 2 = x * x, true); check(x ** 0 = 1, true);",
        ),
        _ => scalar(
            "exp_Real",
            Real,
            "variable rv : real; variable n : integer;",
            "rv := real(W mod 7) + 1.5; n := 2;",
            "check(rv ** n = rv * rv, true); check(rv ** (-n) = 1.0 / (rv * rv), true);",
            "check(rv ** (n - 1) = rv, true); check(rv ** (n - 2) = 1.0, true);",
        ),
    }]
}

/// `**`, `time * real` and `time / real` inside functions the loop calls (native frames of
/// compiled code), against the same operations in line.
fn call_rows() -> Vec<Row> {
    vec![scalar(
        "call_Arith",
        TypeClass::Phys,
        "variable t : time; variable h, rv : real; variable x, n : integer;
    function tm(a : time; b : real) return time is begin return a * b; end function;
    function td(a : time; b : real) return time is begin return a / b; end function;
    function ip(a, b : integer) return integer is begin return a ** b; end function;
    function rp(a : real; b : integer) return real is begin return a ** b; end function;",
        "t := (W mod 7 + 2) * 10 ns; h := 0.5; rv := real(W mod 7) + 1.5; x := W mod 7 + 3; n := 3;",
        "check(tm(t, h) = t / 2, true); check(td(t, h) = t + t, true);
        check(ip(x, n) = x * x * x, true); check(rp(rv, n) = rv * rv * rv, true);",
        "check(tm(t, 1.0) = t, true); check(td(t, 2.0) = t / 2, true);
        check(ip(x, 0) = 1, true); check(rp(rv, 1) = rv, true);",
    )]
}

/// Physical values times and divided by a real.
fn phys_rows(f: Family) -> Vec<Row> {
    let (name, ops1, ops0) = match f {
        Family::Mul => (
            "mul_Phys",
            "check(t * h = t / 2, true); check(t * 2.0 = t + t, true);",
            "check(t * 1.0 = t, true); check(t * 0.0 = 0 ns, true);",
        ),
        _ => (
            "div_Phys",
            "check(t / 2.0 = t / 2, true); check(t / 0.5 = t + t, true);",
            "check(t / 1.0 = t, true); check((t + t) / 2.0 = t, true);",
        ),
    };
    vec![scalar(
        name,
        TypeClass::Phys,
        "variable t : time; variable h : real;",
        "t := (W mod 7 + 2) * 10 ns; h := 0.5;",
        ops1,
        ops0,
    )]
}

/// The design of `row`: `W` a generic, or with `stat` the locally static constant `stat`.
fn design(row: &Row, stat: Option<u32>) -> String {
    let (generics, w) = match stat {
        None => (
            "W : natural := 64; R : natural := 1".to_string(),
            String::new(),
        ),
        Some(w) => (
            "R : natural := 1".to_string(),
            format!("constant W : natural := {w};"),
        ),
    };
    format!(
        "library ieee; use ieee.std_logic_1164.all;
package p is
  {decls}
end package;

library ieee; use ieee.std_logic_1164.all; {uses}
use work.p.all;
entity top is generic ({generics}); end entity;
architecture a of top is
  {w}
begin
  process
    variable fails : integer := 0;
    procedure check(got, want : boolean) is
    begin
      if got /= want then fails := fails + 1; end if;
    end procedure;
    {vars}
  begin
    {setup}
    for k in 1 to R loop
      if k mod 2 = 1 then
        {ops1}
      else
        {ops0}
      end if;
    end loop;
    report \"fails=\" & integer'image(fails);
    wait;
  end process;
end architecture;
",
        decls = row.decls,
        uses = row.uses,
        vars = row.vars,
        setup = row.setup,
        ops1 = row.ops1,
        ops0 = row.ops0,
    )
}

fn yavs(dir: &Path, args: &[&str]) -> (String, BTreeMap<String, u64>) {
    let o = Command::new(env!("CARGO_BIN_EXE_yavs"))
        .current_dir(dir)
        .env("YAVS_COUNTERS", "1")
        // The cost of the operation, not of the memo of pure calls that may answer it.
        .env("YAVS_NO_PMEMO", "1")
        .args(["--color", "never"])
        .args(args)
        .output()
        .expect("run yavs");
    let out = format!(
        "{}{}",
        String::from_utf8_lossy(&o.stdout),
        String::from_utf8_lossy(&o.stderr)
    );
    assert_eq!(o.status.code(), Some(0), "yavs {args:?}:\n{out}");
    let counters = out
        .lines()
        .filter_map(|l| l.strip_prefix("counters: ")?.rsplit_once('='))
        .map(|(k, v)| (k.to_string(), v.parse().expect("counter value")))
        .collect();
    (out, counters)
}

/// Counters per iteration of `row` under `tier` at each width of `WIDTHS`, `W` a generic or with
/// `stat` a constant; the results checked.
fn per_op(row: &Row, tier: &str, stat: bool) -> Vec<BTreeMap<String, u64>> {
    let d = tempdir(&row.name);
    let tier = format!("--tier={tier}");
    WIDTHS
        .iter()
        .map(|&w| {
            let lib = d.join(if stat {
                format!("lib{w}")
            } else {
                "lib".into()
            });
            let work = format!("work:{}", lib.display());
            let file = format!("t{w}.vhd");
            if stat || w == WIDTHS[0] {
                std::fs::write(d.join(&file), design(row, stat.then_some(w))).expect("write");
                yavs(&d, &["analyze", "--std=2008", "--work", &work, &file]);
            }
            let [one, three] = [1, 3].map(|r| {
                let (gw, gr) = (format!("-gW={w}"), format!("-gR={r}"));
                let mut args = vec![
                    "run",
                    "--work",
                    &work,
                    &tier,
                    "--no-cache",
                    "--threads",
                    "1",
                    &gr,
                ];
                if !stat {
                    args.push(&gw);
                }
                args.push("top");
                let (out, c) = yavs(&d, &args);
                assert!(
                    out.contains("fails=0"),
                    "{} W={w}{}: wrong results:\n{out}",
                    row.name,
                    if stat { " (static)" } else { "" }
                );
                c
            });
            three
                .iter()
                .filter_map(|(k, b)| Some((k.clone(), b.saturating_sub(*one.get(k)?) / 2)))
                .collect()
        })
        .collect()
}

/// The measurements of every row under `tier`: generic widths for every row, static widths for
/// `Word` rows, `JOBS` designs at a time. `(row, static, counters per width)`.
type Measured<'a> = Vec<(&'a Row, bool, Vec<BTreeMap<String, u64>>)>;

fn measure<'a>(rows: &'a [Row], tier: &str) -> Measured<'a> {
    let jobs: Vec<(&Row, bool)> = rows
        .iter()
        .flat_map(|r| {
            let s: &[bool] = if r.static_only {
                &[true]
            } else if r.cost == Cost::Word {
                &[false, true]
            } else {
                &[false]
            };
            s.iter().map(move |&s| (r, s))
        })
        .collect();
    let next = AtomicUsize::new(0);
    let out = Mutex::new(Vec::new());
    std::thread::scope(|sc| {
        for _ in 0..JOBS {
            sc.spawn(|| {
                while let Some(&(r, s)) = jobs.get(next.fetch_add(1, Ordering::Relaxed)) {
                    let v = per_op(r, tier, s);
                    out.lock().expect("lock").push((r, s, v));
                }
            });
        }
    });
    let mut v = out.into_inner().expect("lock");
    v.sort_by(|a, b| (&a.0.name, a.1).cmp(&(&b.0.name, b.1)));
    v
}

fn label(row: &Row, stat: bool) -> String {
    format!("{}{}", row.name, if stat { " (static W)" } else { "" })
}

#[test]
fn every_pair_is_decided() {
    // `cell` matches every pair with no wildcard; this runs it on each and keeps the names unique.
    let print = std::env::var_os("YAVS_PERF_PRINT").is_some();
    let mut names = Vec::new();
    for i in all_items() {
        match i {
            Item::Row(r) => names.push(r.name),
            Item::Pending(why) | Item::Later(_, why) => {
                assert!(!why.is_empty(), "an open item says why it is open");
                if print {
                    println!("ops_matrix: open: {why}");
                }
            }
            Item::NotApplicable => {}
        }
    }
    names.sort();
    let n = names.len();
    names.dedup();
    assert_eq!(names.len(), n, "row names are unique");
}

/// A step that landed leaves nothing `Later` for itself: its rows guard it.
#[test]
fn nothing_left_for_a_landed_step() {
    let left: Vec<String> = ALL_FAMILIES
        .iter()
        .flat_map(|&f| {
            ALL_CLASSES
                .iter()
                .map(move |&c| (format!("{f:?} x {c:?}"), cell(f, c)))
        })
        .chain([("construct".to_string(), constructs())])
        .flat_map(|(at, items)| {
            items.into_iter().filter_map(move |i| match i {
                Item::Later(s, why) if s <= LANDED => Some(format!("{at}: step {s}: {why}")),
                Item::Row(r) => match r.step_ok {
                    Some((s, why)) if s <= LANDED => {
                        Some(format!("{at}: row {}: step {s}: {why}", r.name))
                    }
                    _ => None,
                },
                _ => None,
            })
        })
        .collect();
    assert!(
        left.is_empty(),
        "items of landed steps still not guarded by rows:\n  {}",
        left.join("\n  ")
    );
}

/// The slope of every row and the symmetry of every group, in one test so the groups compare.
#[test]
fn interpreter_slopes() {
    let rows = all_rows();
    let measured = measure(&rows, "interp");
    let print = std::env::var_os("YAVS_PERF_PRINT").is_some();
    let mut failed = Vec::new();
    let mut slopes = Vec::new();
    for (row, stat, v) in &measured {
        let at = label(row, *stat);
        let ops = |i: usize| v[i].get("ops").map(|x| x / row.n_ops);
        let (Some(lo), Some(hi)) = (ops(1), ops(2)) else {
            if cfg!(debug_assertions) {
                panic!("{at}: no `ops` counter");
            }
            continue;
        };
        let slope = hi.saturating_sub(lo);
        if print {
            println!(
                "ops_matrix: {at} {:?} ops/op W=64 {lo} W=2048 {hi} slope {slope}",
                row.class
            );
        }
        if slope > WORD_SLOPE {
            failed.push(format!(
                "{at}: {slope} interpreter ops more per operation at W=2048 than at W=64 \
                 (W=64: {lo}, W=2048: {hi}), above the {WORD_SLOPE} of a word-wise row: the \
                 operator lost its fast path"
            ));
        }
        if row.cost == Cost::Word {
            for (i, w) in WIDTHS.iter().enumerate() {
                if let Some(&n) = v[i].get("implicit_elems").filter(|&&n| n > 0) {
                    failed.push(format!(
                        "{at} W={w}: `implicit_elems` {n} per iteration: a `yavs.implicit` native \
                         decoded the elements one by one"
                    ));
                }
                for (k, n) in &v[i] {
                    let general = k.starts_with("ieee_general.") && *n > 0;
                    if general && (*w <= 64 || row.general_ok.is_none()) {
                        failed.push(format!(
                            "{at} W={w}: `{k}` {n} per iteration: an `ieee` native ran its \
                             general path"
                        ));
                    }
                }
            }
        }
        slopes.push((*row, at, slope.max(WORD_SLOPE)));
    }
    for (a, la, sa) in &slopes {
        for (b, lb, sb) in &slopes {
            let ok = a.asym_ok.iter().any(|(n, _)| *n == b.name.as_str());
            if a.group == b.group && !a.group.is_empty() && !ok && *sa > 4 * *sb {
                failed.push(format!(
                    "{la} (slope {sa}) and its sibling {lb} (slope {sb}) differ by more than 4x: \
                     one of them lacks the other's fast path (or list it in `asym_ok` with the \
                     reason)"
                ));
            }
        }
    }
    assert!(failed.is_empty(), "\n  {}", failed.join("\n  "));
}

/// Under the Cranelift tier (compiled before the run): no operation steps to the host
/// (`cb_step`, `cb_xstep` per iteration, debug builds) at any width, and a `Word` row makes no
/// native callback at W of 64 or less and at most one per operation above.
#[cfg(feature = "clift")]
#[test]
fn compiled_host_steps() {
    let rows = all_rows();
    let mut failed = Vec::new();
    for (row, stat, v) in measure(&rows, "clift") {
        let at = label(row, stat);
        for (w, c) in WIDTHS.iter().zip(&v) {
            let n = |k: &str| c.get(k).copied().unwrap_or(0);
            for cb in ["cb_step", "cb_xstep"] {
                if n(cb) != 0 && !(stat && row.step_ok.is_some()) {
                    failed.push(format!("{at} W={w}: {} `{cb}`", n(cb)));
                }
            }
            let native = n("cb_native");
            let bound = if *w <= 64 && !row.native {
                0
            } else {
                row.n_ops
            };
            if row.cost == Cost::Word && native > bound {
                failed.push(format!(
                    "{at} W={w}: {native} `cb_native`, more than {bound} per iteration"
                ));
            }
        }
    }
    assert!(
        failed.is_empty(),
        "callbacks of compiled code per iteration, operators that call the host:\n  {}",
        failed.join("\n  ")
    );
}

#!/usr/bin/env python3
"""Generate VHDL testbenches that exercise std.standard/textio/env value
conversion, run them on sim1 (2008 and 2019) and write golden files.

Usage: gen.py [--sim2]   (run from anywhere)
The command of a label is key golden_cmd of its section in tools/sims.local.conf (bash; env STD, RUNDIR, VHD,
TOP); key golden_name is the label's prefix of the golden file names (default: the label).

Golden format, one case per line:
    func <TAB> arg <TAB> arg ... <TAB> [result]
Arguments are printable ASCII with escapes \\\\, \\t and \\xNN. The result is the
raw (Latin-1) bytes the simulator produced, or the bare word ERROR when the
case aborted the simulation. A result never contains LF.

Simulations run under `timeout 300 systemd-run --user --scope -p
MemoryMax=2G`, at most 8 at a time, in ~/.cache/rsim-std.
"""

import concurrent.futures as cf
import os
import random
import shutil
import subprocess
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[3] / "tools"))
import simconf  # noqa: E402

HERE = Path(__file__).resolve().parent
CACHE = Path.home() / ".cache" / "yavs-std"
JOBS = 8

STDS = ["2008", "2019"]


# ---------------------------------------------------------------- helpers


def vstr(b: bytes) -> str:
    """VHDL expression of type STRING for the given bytes."""
    if len(b) == 0:
        return '""'
    parts = []
    cur = ""
    for c in b:
        if 32 <= c < 127:
            cur += '""' if c == 34 else chr(c)
        else:
            if cur:
                parts.append('"' + cur + '"')
                cur = ""
            parts.append("character'val(%d)" % c)
    if cur:
        parts.append('"' + cur + '"')
    if len(parts) == 1 and parts[0].startswith("character"):
        return "(1 => %s)" % parts[0]
    return "string'(" + " & ".join(parts) + ")" if len(parts) > 1 else parts[0]


def esc(a) -> str:
    if isinstance(a, bytes):
        out = ""
        for c in a:
            if c == 92:
                out += "\\\\"
            elif c == 9:
                out += "\\t"
            elif 32 <= c < 127:
                out += chr(c)
            else:
                out += "\\x%02x" % c
        return out
    return str(a)


class Case:
    def __init__(self, func, args, body, decls="", alone=False, stds=None):
        self.func = func
        self.args = args
        self.body = body  # statements; must call emit(K, <string>)
        self.decls = decls
        self.alone = alone  # run in its own simulation (may abort)
        self.stds = stds or STDS


def int_min(std):
    return "(-2147483647 - 1)" if std == "2008" else "(-9223372036854775807 - 1)"


def fs(n: int) -> str:
    if n == -(2**63):
        return "time'low"
    if n < 0:
        return "(-%d fs)" % -n
    return "(%d fs)" % n


SIDES = ["right", "left"]
TIME_UNITS = ["fs", "ps", "ns", "us", "ms", "sec", "min", "hr"]

REALS = [
    "0.0", "-0.0", "1.0", "-1.0", "0.1", "0.5", "1.5", "2.5", "-2.5", "3.5",
    "0.333333333333333314829616256247", "123.456", "-123.456", "1.0e-5",
    "1.0e-4", "0.001", "0.0001", "1.0e15", "1.0e16", "1.0e17", "123456789.0",
    "1.0e100", "1.0e-100", "1.7976931348623157e308", "-1.7976931348623157e308",
    "2.2250738585072014e-308", "4.9406564584124654e-324", "9007199254740993.0",
    "100.0", "1.0e21", "1.0e22", "3.14159265358979", "0.000123456789",
    "999999999999999.0", "9999999999999999.0", "0.05", "0.25", "1.0e-10",
    "12345.678901234567", "1.0e14", "1.5e-7", "6.02214076e23", "1.0e300",
]
rng = random.Random(1076)
for _ in range(60):
    x = rng.uniform(-1, 1) * 10 ** rng.randint(-30, 30)
    REALS.append("%.17e" % x)
for _ in range(20):
    REALS.append("%.17e" % (rng.randint(-10**6, 10**6) / 8.0))

FORMATS = [
    "%f", "%.3f", "%10.3f", "%-10.3f|", "%e", "%.2e", "%E", "%g", "%.3g", "%G",
    "%+f", "% f", "%010.2f", "%#g", "%#.0f", "%.0f", "%.0e", "%.10g", "%.17g",
    "%12.4E", "x=%6.2f!", "%%%.1f", "%-+12.3e", "%#.3e", "%.0g", "%#.0g",
    "%.20f", "%5.1g",
]
FORMAT_REALS = ["0.0", "-0.0", "1.0", "-1.5", "0.1", "123.456", "1.0e-5",
                "1.0e100", "2.5", "0.5", "100000.0", "1234567.0", "9.9999",
                "0.00001234", "-987654.321"]
DIGITS = [0, 1, 2, 3, 6, 10, 15, 17, 20, 30]

INTS32 = [0, 1, -1, 7, 10, 42, -42, 100, 999, 1000, -1000, 65535, 123456789,
          2147483647, -2147483647, 1 << 30]
INTS64 = [2**31, -(2**31) - 1, 2**40, -(2**40), 2**62, 2**63 - 1,
          -(2**63) + 1, 1234567890123456789]

TIMES = [0, 1, -1, 999, 1000, 1500, 1000000, 5000000, 1500000, 123456789,
         10**12, 10**15, 60 * 10**15, 3600 * 10**15, 2**63 - 1, -(2**63),
         -(2**63) + 1, -5000000, 1234567890123456789, 7, 1001, 999999999]

# enumeration type used by the enumeration cases (pos -> source text)
E1 = ["Foo", "\\Bar\\", "'x'", "bAz", "\\a\\\\b\\", "Q", "'Q'", "\\X\\"]
E1_DECL = "type e1 is (%s);" % ", ".join(E1)

DIST_DECL = """type dist is range -1000000000 to 1000000000 units
    nm; um = 1000 nm; mm = 1000 um; m = 1000 mm; Inch = 25400 um; \\Ft\\ = 12 Inch;
  end units;"""


def bulk_cases():
    cases = []
    add = cases.append
    for std in STDS:
        ints = INTS32 + (INTS64 if std == "2019" else [])
        for v in ints:
            lit = str(v)
            if v == -(2**31) and std == "2008" or v == -(2**63):
                lit = int_min(std)
            elif v < 0:
                lit = "(%s)" % lit
            add(Case("int_image", [v], f"emit(K, integer'image({lit}));", stds=[std]))
            add(Case("int_to_string", [v], f"emit(K, to_string({lit}));", stds=[std]))
        add(Case("int_image", [-(2**31) if std == "2008" else -(2**63)],
                 "emit(K, integer'image(integer'low));", stds=[std]))
    for r in REALS:
        add(Case("real_image", [r], f"emit(K, real'image({r}));"))
        add(Case("real_to_string", [r], f"emit(K, to_string({r}));"))
    for r in REALS[:40]:
        for d in DIGITS:
            add(Case("real_digits", [r, d], f"emit(K, to_string({r}, {d}));"))
    for r in FORMAT_REALS:
        for f in FORMATS:
            add(Case("real_format", [r, f.encode()], f"emit(K, to_string({r}, {vstr(f.encode())}));"))
    for t in TIMES:
        add(Case("time_image", [t], f"emit(K, time'image({fs(t)}));"))
        add(Case("time_to_string", [t], f"emit(K, to_string({fs(t)}));"))
        for u in TIME_UNITS:
            add(Case("time_unit", [t, u], f"emit(K, to_string({fs(t)}, {u}));"))
    for v in [0, 1, -1, 1000, 1500000, 25400000, -304800000, 1000000000]:
        add(Case("dist_image", [v], f"emit(K, dist'image(dist'({v} nm)));" if v >= 0
                 else f"emit(K, dist'image(-dist'({-v} nm)));"))
    for pos in range(len(E1)):
        add(Case("e1_image", [pos], f"emit(K, e1'image(e1'val({pos})));"))
        add(Case("e1_to_string", [pos], f"emit(K, to_string(e1'val({pos})));"))
    for c in range(256):
        add(Case("char_image", [c], f"emit(K, character'image(character'val({c})));"))
        add(Case("char_to_string", [c], f"emit(K, to_string(character'val({c})));"))
    for b in ["false", "true"]:
        add(Case("bool_image", [b], f"emit(K, boolean'image({b}));"))
        add(Case("bool_to_string", [b], f"emit(K, to_string({b}));"))
    for b in "01":
        add(Case("bit_image", [b], f"emit(K, bit'image('{b}'));"))
        add(Case("bit_to_string", [b], f"emit(K, to_string(bit'('{b}')));"))
    for s in ["note", "warning", "error", "failure"]:
        add(Case("sev_image", [s], f"emit(K, severity_level'image({s}));"))
    bvs = ["", "0", "1", "10", "101", "1010", "11010", "11111111", "100000000",
           "0000", "000000001", "1111111111111", "10101010101010101010101"]
    for bv in bvs:
        lit = 'bit_vector\'("%s")' % bv
        for fn in ["to_string", "to_hstring", "to_ostring", "to_bstring"]:
            add(Case("bv_" + fn, [bv], f"emit(K, {fn}({lit}));"))
    # strings: 'image of a string is not defined; to_string(string) does not
    # exist either, so only justify covers string formatting.
    for s in [b"", b"a", b"abc", b"hello world"]:
        for side in SIDES:
            for f in [0, 1, 3, 5, 12]:
                add(Case("justify", [s, side, f], f"emit(K, justify({vstr(s)}, {side}, {f}));"))
    cases += textio_cases()
    return cases


def rd(func, ty, line: bytes, n=None, img=None, call="read", extra_decl=""):
    """A textio read case: put LINE in a line, read, show good/value/rest."""
    decl = f"variable v : {ty};" + extra_decl
    if img is None:
        img = f"{ty}'image(v)"
    body = f"""l := new string'({vstr(line)});
    {call}(l, v, ok);
    if ok then s := new string'("g=true v=" & {img});
    else s := new string'("g=false"); end if;
    if l = null then emit(K, s.all & " rest=<null>");
    else emit(K, s.all & " rest=" & l.all); end if;"""
    args = [line] + ([n] if n is not None else [])
    return Case(func, args, body, decls=decl)


def wr(func, args, call):
    return Case(func, args, f"{call}\n    if l = null then emit(K, \"<null>\"); else emit(K, l.all); end if;")


def textio_cases():
    cases = []
    add = cases.append
    ws = [b"", b" ", b"\t"]
    int_lines = [b"42", b"  42", b"\t42 rest", b"-42", b"+42", b"- 42", b"1_000",
                 b"1__0", b"_1", b"1_", b"1e3", b"1E3x", b"16#FF#", b"2147483647",
                 b"2147483648", b"-2147483648", b"-2147483649",
                 b"9223372036854775807", b"9223372036854775808", b"99999999999999999999",
                 b"", b"   ", b"abc", b"12abc", b"1.5", b"0x10", b"007", b"-",
                 b"1 2 3", b"\xa042", b"42\r", b"  -0 "]
    for s in int_lines:
        add(rd("read_int", "integer", s))
    real_lines = [b"1.0", b"  -2.5e3 x", b"1", b"1.", b".5", b"1.5e", b"1.5e+",
                  b"1_000.5", b"2#1.1#E2", b"16#F.8#", b"1.0e400", b"1.0e-400",
                  b"-0.0", b"3.14159265358979323846", b"+1.0", b"", b"abc",
                  b"1.0.0", b"1e5", b"1.0E-2", b"0.1"]
    for s in real_lines:
        add(rd("read_real", "real", s))
    time_lines = [b"5 ns", b"5ns", b"  1.5 ns rest", b"1.5 NS", b"ns", b"5 xs", b"5",
                  b"-5 ns", b"0.1 fs", b"1.0e3 ps", b"16#A# ns", b"5  us",
                  b"9223372036854775807 fs", b"9223372036854775808 fs", b"10000000 hr",
                  b"1 sec", b"2 min", b"1 hr", b"1.25 us", b"5 nsx", b"5 n", b"",
                  b"1_0 ps", b"\t3 ms\t", b"0.0000001 ns", b"-1.5 ps"]
    for s in time_lines:
        add(rd("read_time", "time", s))
    for s in [b"true", b"TRUE", b" False ", b"truex", b"t", b"", b"1", b"tru", b"false,"]:
        add(rd("read_bool", "boolean", s))
    for s in [b"0", b"1", b" 1", b"x", b"", b"'1'", b"10", b"\t0"]:
        add(rd("read_bit", "bit", s))
    for s in [b"a", b" a", b"", b"ab", b"\t", b"\xe9"]:
        add(rd("read_char", "character", s, img="character'image(v)"))
    for s, n in [(b"abc", 3), (b"abcdef", 3), (b"ab", 3), (b"  abc", 3), (b"", 2), (b"abc", 0)]:
        add(rd("read_str", f"string(1 to {n})", s, n=n, img="v"))
    for s, n in [(b"abc def", 10), (b"  abc def", 10), (b"abcdef", 3), (b"", 5),
                 (b"\tab\tc", 5), (b"   ", 4), (b"abc", 0)]:
        add(Case("sread", [s, n], f"""l := new string'({vstr(s)});
    sread(l, v, n);
    if l = null then emit(K, "n=" & integer'image(n) & " v=" & v(1 to n) & " rest=<null>");
    else emit(K, "n=" & integer'image(n) & " v=" & v(1 to n) & " rest=" & l.all); end if;""",
                 decls=f"variable v : string(1 to {n}); variable n : natural;"))
    bv_lines = [(b"1010", 4), (b"10_10", 4), (b"1010", 3), (b"101", 4), (b" 1010", 4),
                (b"1012", 4), (b"10100", 4), (b"", 2), (b"_1010", 4), (b"1__010", 4),
                (b"1010_", 4), (b"11", 0), (b"1 0", 2)]
    for s, n in bv_lines:
        add(rd("read_bv", f"bit_vector(0 to {max(n, 1) - 1})" if n else "bit_vector(0 to -1)", s,
               n=n, img="to_string(v)"))
    hex_lines = [(b"A5", 8), (b"a5", 8), (b"A_5", 8), (b"F", 4), (b"1F", 5), (b"3F", 5),
                 (b"FF", 7), (b"7F", 7), (b"G", 4), (b"  1234", 16), (b"12", 16),
                 (b"123456", 16), (b"", 4), (b"0", 1), (b"1", 1), (b"2", 1), (b"A5x", 8),
                 (b"A__5", 8), (b"_A5", 8), (b"AB", 0)]
    for s, n in hex_lines:
        ty = f"bit_vector(0 to {n - 1})" if n else "bit_vector(0 to -1)"
        add(rd("hread", ty, s, n=n, img="to_string(v)", call="hread"))
    oct_lines = [(b"75", 6), (b"7_5", 6), (b"17", 4), (b"37", 4), (b"8", 3), (b" 123", 9),
                 (b"12", 9), (b"", 3), (b"3", 2), (b"4", 2), (b"777", 9)]
    for s, n in oct_lines:
        add(rd("oread", f"bit_vector(0 to {n - 1})", s, n=n, img="to_string(v)", call="oread"))
    for s, n in [(b"1010", 4), (b"10_10", 4), (b"x", 1)]:
        add(rd("bread", f"bit_vector(0 to {n - 1})", s, n=n, img="to_string(v)", call="bread"))
    # sequential reads from one line
    add(Case("read_int_seq", [b"1 2  -3\t4"], """l := new string'("1 2  -3" & HT & "4");
    read(l, a, ok); read(l, b, ok); read(l, c, ok); read(l, d, ok);
    emit(K, integer'image(a) & "," & integer'image(b) & "," & integer'image(c) & "," & integer'image(d) & " rest=" & l.all);""",
             decls="variable a, b, c, d : integer;"))

    # writes
    for v in [0, 42, -42, 2147483647]:
        for side in SIDES:
            for f in [0, 2, 6]:
                add(wr("write_int", [v, side, f], f"write(l, integer'({v if v >= 0 else '(' + str(v) + ')'}), {side}, {f});"))
    for r in ["0.0", "1.0", "-1.5", "0.333333333333333314829616256247", "123.456", "1.0e-20",
              "1.0e100", "2.5", "1.0e15", "-0.0", "0.5", "1.0e16"]:
        for side, f, d in [("right", 0, 0), ("right", 15, 0), ("left", 15, 0), ("right", 0, 1),
                           ("right", 0, 3), ("right", 10, 2), ("left", 10, 2), ("right", 0, 20),
                           ("right", 3, 5)]:
            add(wr("write_real", [r, side, f, d], f"write(l, real'({r}), {side}, {f}, {d});"))
        for fmt in ["%.3f", "%e", "%10.2f", "%g"]:
            add(wr("write_real_format", [r, fmt.encode()], f"write(l, real'({r}), {vstr(fmt.encode())});"))
    for t in [0, 1, 1500, 5000000, 1500000, -1, 10**15, 2**63 - 1, -(2**63), 123456789]:
        for side, f, u in [("right", 0, "ns"), ("left", 14, "ns"), ("right", 14, "ps"),
                           ("right", 0, "fs"), ("right", 0, "us"), ("right", 0, "sec"),
                           ("right", 0, "hr"), ("right", 0, "ms"), ("right", 0, "min")]:
            add(wr("write_time", [t, side, f, u], f"write(l, {fs(t)}, {side}, {f}, {u});"))
    for s in [b"", b"abc", b"a b"]:
        for side, f in [("right", 0), ("right", 5), ("left", 5), ("right", 2)]:
            add(wr("write_str", [s, side, f], f"write(l, string'({vstr(s)}), {side}, {f});"))
            add(wr("swrite", [s, side, f], f"swrite(l, {vstr(s)}, {side}, {f});"))
    for side, f in [("right", 0), ("right", 3), ("left", 3)]:
        add(wr("write_bit", ["1", side, f], f"write(l, bit'('1'), {side}, {f});"))
        add(wr("write_bool", ["true", side, f], f"write(l, true, {side}, {f});"))
        add(wr("write_bool", ["false", side, f], f"write(l, false, {side}, {f});"))
        add(wr("write_char", [ord("a"), side, f], f"write(l, character'('a'), {side}, {f});"))
        add(wr("write_char", [9, side, f], f"write(l, HT, {side}, {f});"))
    for bv in ["", "1", "1010", "11010", "101010101", "1111111111111"]:
        lit = 'bit_vector\'("%s")' % bv
        for side, f in [("right", 0), ("right", 8), ("left", 8), ("right", 1)]:
            add(wr("write_bv", [bv, side, f], f"write(l, {lit}, {side}, {f});"))
            add(wr("bwrite", [bv, side, f], f"bwrite(l, {lit}, {side}, {f});"))
            add(wr("hwrite", [bv, side, f], f"hwrite(l, {lit}, {side}, {f});"))
            add(wr("owrite", [bv, side, f], f"owrite(l, {lit}, {side}, {f});"))
    # writes append to an existing line
    add(wr("write_append", [], 'write(l, string\'("ab")); write(l, 12, right, 4); write(l, bit\'(\'1\'));'))
    return cases


VALUE_CASES = []


def vcase(func, ty, s: bytes, img=None, stds=None):
    if img is None:
        img = f"{ty}'image(v)"
    VALUE_CASES.append(Case(func, [s], f"v := {ty}'value({vstr(s)}); emit(K, {img});",
                            decls=f"variable v : {ty};", alone=True, stds=stds))


INT_VALUES = [b"42", b"  -42  ", b"+5", b"- 5", b"1_000", b"1__0", b"_1", b"1_", b"1e3",
              b"1E3", b"1e-3", b"16#FF#", b"16#ff#", b"2#1010#E3", b"16:FF:", b"8#777#",
              b"36#Z#", b"37#1#", b"1#0#", b"16#G#", b"16#FF", b"", b" ", b"0x10", b"1.0",
              b"2147483647", b"2147483648", b"-2147483648", b"-2147483649",
              b"9223372036854775807", b"9223372036854775808", b"-9223372036854775808",
              b"99999999999999999999", b"1 2", b"5 garbage", b"\t5\t", b"\xa05",
              b"5\r", b"5\n", b"007", b"-0", b"1e10", b"1e20", b"16#_F#", b"16#F_F#",
              b"2#1_0#"]
REAL_VALUES = [b"1.0", b"1", b"1.5e3", b"-2.5", b"1.0E+2", b"2#1.1#E2", b"16#F.8#",
               b".5", b"5.", b"1_000.5", b"inf", b"nan", b"1.0e400", b"1.0e-400",
               b"  3.25  ", b"1.5e", b"1.0 x", b"+1.0", b"1.0e-5", b"0.1", b"1e5",
               b"3.14159265358979323846264338327950288", b"16#1.0#E-2", b"2#0.1#",
               b"10#1.5#", b"1.7976931348623157e308", b"1.7976931348623159e308",
               b"4.9406564584124654e-324", b"2.4703282292062328e-324", b"1.0E+0_1"]
TIME_VALUES = [b"5 ns", b"5ns", b"1.5 ns", b"1.5 NS", b" 2 us ", b"ns", b"0.1 fs",
               b"1.0e3 ps", b"16#A# ns", b"5 xs", b"5", b"-5 ns", b"- 5 ns",
               b"9223372036854775807 fs", b"9223372036854775808 fs", b"-9223372036854775808 fs",
               b"10000000 hr", b"1.25 us", b"1.5 fs", b"2.5 fs", b"-1.5 fs", b"5\tns",
               b"5  ns", b"1 Sec", b"1.0 hr", b"0.5 sec", b"", b"1e3 fs",
               b"123456789.123456789 ns", b"1.9999999 fs"]
E1_VALUES = [b"foo", b"FOO", b" Foo ", b"\\Bar\\", b"\\bar\\", b"\\BAR\\", b"'x'", b"x",
             b"'X'", b"baz", b"\\a\\\\b\\", b"q", b"'Q'", b"'q'", b"\\X\\", b"\\x\\", b"X",
             b"Fo o", b"foo bar", b"", b"''x''", b"foo\t"]
CHAR_VALUES = [b"'a'", b"NUL", b"nul", b"'''", b"a", b"' '", b"'\xe9'", b"C128", b"c128",
               b"del", b"''", b"'ab'", b" 'b' ", b"LF"]
BOOL_VALUES = [b"true", b"TRUE", b"False", b" true ", b"t", b"'1'", b"1"]
BIT_VALUES = [b"'0'", b"'1'", b"1", b" '1' ", b"'2'"]
DIST_VALUES = [b"5 nm", b"1 um", b"2 Inch", b"2 inch", b"1 \\Ft\\", b"1 \\ft\\", b"1 ft",
               b"1.5 mm", b"m", b"0.0001 nm", b"-3 mm", b"2000000 mm"]
SEV_VALUES = [b"note", b"Warning", b"FAILURE", b"fatal"]


def value_cases():
    for s in INT_VALUES:
        vcase("int_value", "integer", s)
    for s in REAL_VALUES:
        vcase("real_value", "real", s)
    for s in TIME_VALUES:
        vcase("time_value", "time", s)
    for s in E1_VALUES:
        vcase("e1_value", "e1", s)
    for s in CHAR_VALUES:
        vcase("char_value", "character", s)
    for s in BOOL_VALUES:
        vcase("bool_value", "boolean", s)
    for s in BIT_VALUES:
        vcase("bit_value", "bit", s)
    for s in DIST_VALUES:
        vcase("dist_value", "dist", s)
    for s in SEV_VALUES:
        vcase("sev_value", "severity_level", s)
    # real'image(x) of odd formats, the digits/format argument errors
    for r, f in [("1.5", "%d"), ("1.5", "%s"), ("1.5", "abc"), ("1.5", ""), ("1.5", "%f%f"),
                 ("1.5", "%a"), ("1.5", "%5"), ("1.5", "%.3"), ("1.5", "%lf")]:
        VALUE_CASES.append(Case("real_format", [r, f.encode()],
                                f"emit(K, to_string({r}, {vstr(f.encode())}));", alone=True))
    for t, u in [(5000000, "2 ns"), (5000000, "0 ns"), (5000000, "1.5 ns")]:
        VALUE_CASES.append(Case("time_unit_odd", [t, u.encode()],
                                f"emit(K, to_string({fs(t)}, {u}));", alone=True))
    return VALUE_CASES


# readline / writeline / endfile over files written byte-exactly by Python
READLINE_FILES = [
    b"", b"a", b"a\n", b"a\nb", b"a\nb\n", b"\n", b"\n\n", b"a\r\nb\r\n", b"a\rb\n",
    b"a\r\r\nb", b"a\r", b"abc def\n\nxyz\n", b"\xe9\xff\n", b"a\tb\n", b"\r\n",
    b"x" * 300 + b"\n",
]


def make_readline_cases():
    cases = []
    for i, data in enumerate(READLINE_FILES):
        body = f"""file_open(st, f, "rl{i}.txt", read_mode);
    while not endfile(f) loop
      readline(f, l);
      write(acc, string'("<"));
      write(acc, integer'image(l'length));
      write(acc, string'(":"));
      for j in 1 to l'length loop
        c := l(j);
        if character'pos(c) < 32 or character'pos(c) > 126 then
          write(acc, string'("\\") & integer'image(character'pos(c)));
        else
          write(acc, c);
        end if;
      end loop;
      write(acc, string'(">"));
      deallocate(l);
    end loop;
    file_close(f);
    emit(K, acc.all);"""
        decls = ("file f : text; variable st : file_open_status; "
                 "variable acc : line := new string'(\"\"); variable c : character;")
        cases.append(Case("readline", [data], body, decls))
    return cases


WRITELINE_CASES = [
    ("writeline", [b"ab"], 'write(l, string\'("ab")); writeline(f, l);'),
    ("writeline", [b"", b"x"], 'writeline(f, l); write(l, string\'("x")); writeline(f, l);'),
    ("writeline", [b"a\rb"], 'write(l, string\'("a") & CR & "b"); writeline(f, l);'),
    ("writeline", [b"a\nb"], 'write(l, string\'("a") & LF & "b"); writeline(f, l);'),
]


def make_writeline_cases():
    cases = []
    for i, (fn, args, stmts) in enumerate(WRITELINE_CASES):
        body = f"""file_open(f, "wl{i}.txt", write_mode);
    {stmts}
    file_close(f);
    file_open(g, "wl{i}.txt", read_mode);
    n := 0;
    while not endfile(g) loop
      read(g, c);
      n := n + 1;
      write(acc, integer'image(character'pos(c)) & " ");
    end loop;
    file_close(g);
    emit(K, "null=" & boolean'image(l = null) & " bytes=" & acc.all);"""
        decls = ("file f : text; type cf is file of character; file g : cf; variable c : character; "
                 "variable n : natural; variable acc : line := new string'(\"\");")
        cases.append(Case(fn, [esc(a) for a in args], body, decls))
    return cases


def env_cases():
    cases = []
    add = cases.append
    add(Case("resolution_limit", [], "emit(K, time'image(std.env.resolution_limit));"))
    s19 = ["2019"]
    add(Case("vhdl_version", [], "emit(K, std.env.vhdl_version);", stds=s19))
    for nm in ["tool_type", "tool_vendor", "tool_name", "tool_edition", "tool_version"]:
        add(Case(nm, [], f"emit(K, std.env.{nm});", stds=s19))
    add(Case("dir_separator", [], "emit(K, std.env.dir_separator);", stds=s19))
    epochs = ["0.0", "1.0", "86399.999999", "86400.0", "951782400.0", "951868800.0",
              "1700000000.123456", "1700000000.9999996", "-1.0", "-86400.5", "4102444800.0",
              "1234567890.5", "68169600.0", "253402300799.0", "1709251199.25", "1.0e9"]
    for e in epochs:
        for d in [0, 3, 6]:
            add(Case("gmtime_to_string", [e, d], f"emit(K, to_string(gmtime({e}), {d}));", stds=s19))
        add(Case("gmtime_fields", [e], f"""tr := gmtime({e});
    emit(K, integer'image(tr.year) & " " & integer'image(tr.month) & " " & integer'image(tr.day) & " " &
            integer'image(tr.hour) & " " & integer'image(tr.minute) & " " & integer'image(tr.second) & " " &
            integer'image(tr.microsecond) & " " & dayofweek'image(tr.weekday) & " " & integer'image(tr.dayofyear));""",
                 decls="variable tr : time_record;", stds=s19))
        add(Case("epoch_gmtime", [e], f"emit(K, real'image(epoch(gmtime({e}))));", stds=s19))
        add(Case("trec_plus", [e, "90061.5"], f"emit(K, to_string(gmtime({e}) + 90061.5, 6));", stds=s19))
        add(Case("trec_minus", [e, "90061.5"], f"emit(K, to_string(gmtime({e}) - 90061.5, 6));", stds=s19))
        add(Case("trec_diff", [e, "1000000.25"], f"emit(K, real'image(gmtime({e}) - gmtime(1000000.25)));", stds=s19))
    for t in [0, 1, 1500, 10**15, 1500 * 10**12, -(10**15), 2**63 - 1, 123456789]:
        add(Case("time_to_seconds", [t], f"emit(K, real'image(time_to_seconds({fs(t)})));", stds=s19))
    for r in ["0.0", "1.0", "1.5e-12", "1.0e-15", "1.4e-15", "1.6e-15", "-2.5", "9223.372036854775807",
              "1.0e-16", "0.1", "3600.0"]:
        add(Case("seconds_to_time", [r], f"emit(K, time'image(seconds_to_time({r})));", stds=s19))
    return cases


def render(cases, std, name, single=None):
    decls = []
    calls = []
    for k, c in enumerate(cases):
        if single is not None and k != single:
            continue
        body = c.body.replace("emit(K,", f"emit({k},")
        decls.append(f"  procedure c{k} is\n    variable l, s : line; variable ok : boolean;\n    {c.decls}\n  begin\n    {body}\n  end procedure;")
        calls.append(f"    c{k};")
    env_use = "use std.env.all;" if std == "2019" else ""
    return f"""-- Generated by gen.py; do not edit.
use std.textio.all;
{env_use}
entity {name} is end entity;
architecture a of {name} is
  {E1_DECL}
  {DIST_DECL}
begin
  process
  procedure emit(k : integer; s : string) is
    variable l : line;
  begin
    write(l, "#" & integer'image(k) & HT & "[" & s & "]");
    writeline(output, l);
  end procedure;
{chr(10).join(decls)}
  begin
{chr(10).join(calls)}
    wait;
  end process;
end architecture;
"""


def sim(tool, std, vhd: Path, top: str, rundir: Path):
    rundir.mkdir(parents=True, exist_ok=True)
    for i, data in enumerate(READLINE_FILES):
        (rundir / f"rl{i}.txt").write_bytes(data)
    cmd = simconf.get(tool, "golden_cmd")
    if cmd is None:
        sys.exit(f"{tool}: key golden_cmd not configured (tools/sims.local.conf)")
    cmd = ["env", f"STD={std}", f"RUNDIR={rundir}", f"VHD={vhd}", f"TOP={top}", "bash", "-c", cmd]
    full = ["timeout", "300", "systemd-run", "--user", "--scope", "-q", "-p", "MemoryMax=2G",
            "--working-directory", str(rundir)] + cmd
    p = subprocess.run(full, cwd=rundir, capture_output=True)
    return p.returncode, p.stdout, p.stderr


def parse_out(out: bytes):
    res = {}
    for line in out.split(b"\n"):
        if line.startswith(b"#") and b"\t[" in line:
            k, rest = line[1:].split(b"\t[", 1)
            if rest.endswith(b"]"):
                res[int(k)] = rest[:-1]
    return res


def golden_line(c, result: bytes) -> bytes:
    parts = [c.func] + [esc(a) for a in c.args]
    return "\t".join(parts).encode() + b"\t" + (b"[" + result + b"]" if result is not None else b"ERROR")


def run_tool(tool, outdir: Path):
    all_cases = bulk_cases() + env_cases() + make_readline_cases() + make_writeline_cases()
    vcases = value_cases()
    stds = STDS if tool == "sim1" else ["2008"]
    for std in stds:
        base = CACHE / tool / std
        if base.exists():
            shutil.rmtree(base)
        base.mkdir(parents=True)
        bulk = [c for c in all_cases if std in c.stds]
        alone = [c for c in vcases if std in c.stds]
        # bulk in chunks; a chunk that fails is re-run one case at a time so
        # that an abort (or a crash of the simulator) only marks that case as ERROR
        chunks = [bulk[i:i + 100] for i in range(0, len(bulk), 100)] + [[c] for c in alone]

        def run_chunk(ci, chunk):
            d = base / f"c{ci}"
            d.mkdir()
            vhd = d / "tb.vhd"
            vhd.write_text(render(chunk, std, "tb"), encoding="latin-1")
            rc, out, _ = sim(tool, std, vhd, "tb", d)
            return rc, parse_out(out)

        with cf.ThreadPoolExecutor(JOBS) as ex:
            results = list(ex.map(lambda a: run_chunk(*a), enumerate(chunks)))
            retry = [(ci, k) for ci, (rc, _) in enumerate(results) if rc != 0 and len(chunks[ci]) > 1
                     for k in range(len(chunks[ci]))]
            singles = list(ex.map(lambda a: run_chunk(f"{a[0]}_{a[1]}", [chunks[a[0]][a[1]]]), retry))
        single = {key: res.get(0) for key, (_, res) in zip(retry, singles)}
        lines = []
        for ci, chunk in enumerate(chunks):
            for k, c in enumerate(chunk):
                r = single[(ci, k)] if (ci, k) in single else results[ci][1].get(k)
                lines.append(golden_line(c, r))
        outdir.mkdir(parents=True, exist_ok=True)
        (outdir / f"{simconf.get(tool, 'golden_name', tool)}-{std}.txt").write_bytes(b"\n".join(lines) + b"\n")
        print(f"{tool} {std}: {len(lines)} cases, {sum(1 for l in lines if l.endswith(b'ERROR'))} errors")


if __name__ == "__main__":
    if "--sim2" in sys.argv:
        run_tool("sim2", CACHE / "sim2-golden")
    else:
        run_tool("sim1", HERE)

"""Derive yavs's lib/std and lib/ieee from a checkout of vasg/Packages.

usage: derive_libs.py <packages-checkout> <outdir>

Everything is byte-for-byte upstream except the documented edits below.
Files are handled as latin-1 so the 8-bit bytes in STANDARD and NUMERIC_* survive.
"""
import os
import re
import sys

src, out = sys.argv[1:3]
NOTE = (
    "-- yavs: derived from the IEEE VASG 1076-2019 package source by\n"
    "-- tools/derive_libs.py (see lib/SOURCES.md for the exact edits).\n"
)


def rd(p):
    with open(os.path.join(src, p), encoding="latin-1", newline="") as f:
        return f.read()


def wr(p, s):
    p = os.path.join(out, p)
    os.makedirs(os.path.dirname(p), exist_ok=True)
    with open(p, "w", encoding="latin-1", newline="") as f:
        f.write(s)


def drop_block(s, start_re, end_re):
    """Delete from the line matching start_re up to (excluding) the line matching end_re."""
    lines = s.split("\n")
    i = next(k for k, l in enumerate(lines) if re.match(start_re, l))
    j = next(k for k in range(i + 1, len(lines)) if re.match(end_re, lines[k]))
    return "\n".join(lines[:i] + lines[j:])


def drop_line(s, pat):
    new, n = re.subn(pat + r"[^\n]*\n", "", s, count=1, flags=re.M)
    assert n == 1, pat
    return new


# ---- std 2019: only the implementation_defined ranges are made concrete -------
std19 = rd("std/standard.vhdl")
for name, rng in (
    # -2^63 is INTEGER'low (the full i64 range, as the reference simulator; #601).
    ("INTEGER", "-9223372036854775808 to 9223372036854775807"),
    ("REAL", "-1.7976931348623157e308 to 1.7976931348623157e308"),
    # -2^63 fs is TIME'low (the full i64 range, as the reference simulator).
    ("TIME", "-9223372036854775808 to 9223372036854775807"),
):
    std19, n = re.subn(
        rf"^(  type {name} is range) implementation_defined", rf"\1 {rng}", std19, flags=re.M
    )
    assert n == 1, name
# Upstream lost two CHARACTER literals in conversion: position 160 (NBSP) and
# 173 (soft hyphen) are plain ' ' and '-', duplicating positions 32 and 45.
std19, n = re.subn(r"^    ' ',  '\xa1',", "    '\xa0',  '\xa1',", std19, flags=re.M)
assert n == 1
std19, n = re.subn(r"'\xac',  '-',  '\xae',", "'\xac',  '\xad',  '\xae',", std19)
assert n == 1
# LRM 1076-2019 16.3 names the direction type RANGE_DIRECTION; the package source
# says DIRECTION (VI#97). Rename the type and its comment references, not the
# record element `Direction`.
std19, n = re.subn(r"\bDIRECTION(?=\b|_RANGE_RECORD)", "RANGE_DIRECTION", std19)
assert n == 29, n
wr("std/2019/standard.vhdl", std19)
wr("std/2019/textio.vhdl", rd("std/textio.vhdl"))
wr("std/2019/env.vhdl", rd("std/env.vhdl"))
wr("std/2019/reflection.vhdl", rd("std/reflection.vhdl"))

# ---- std 2008: drop what 1076-2019 added ---------------------------------------
s = std19
s = drop_block(s, r"  type RANGE_DIRECTION is \(", r"  type BOOLEAN is")
s = drop_line(s, r"    READ_WRITE_MODE,")
s = drop_block(s, r"  type FILE_OPEN_STATE is \(", r"  -- The 'FOREIGN attribute")
s = s.replace(  # 1076-2008 INTEGER is 32 bit
    "type INTEGER is range -9223372036854775808 to 9223372036854775807",
    "type INTEGER is range -2147483648 to 2147483647",
)
wr("std/2008/standard.vhdl", s)

t = rd("std/textio.vhdl")
t = drop_line(t, r" ?type LINE_VECTOR")
wr("std/2008/textio.vhdl", t)

e = rd("std/env.vhdl")
wr(
    "std/2008/env.vhdl",
    e.split("--   Title")[0]
    + NOTE
    + "--\n--   Title     :  ENV package (1076-2008 content)\n--\n\n"
    "package ENV is\n"
    "  procedure STOP (STATUS: INTEGER);\n"
    "  procedure STOP;\n"
    "  procedure FINISH (STATUS: INTEGER);\n"
    "  procedure FINISH;\n"
    "  function RESOLUTION_LIMIT return DELAY_LENGTH;\n"
    "end package ENV;\n",
)

# ---- std 1993: drop what 1076-2008 added ---------------------------------------
# STANDARD: the four predefined vector types (15 lines of implicit-operation comments each
# go with them); the other 2008 additions of STANDARD are implicit operations, made by the
# analyzer per revision. TEXTIO: JUSTIFY, TEE, SREAD, the binary/octal/hex routines with
# their aliases, SWRITE and WRITE of REAL with a format. There is no ENV in 1993.
s8 = s
s8 = drop_block(s8, r"  type BOOLEAN_VECTOR is", r"  type BIT_VECTOR is")
s8 = drop_block(s8, r"  type INTEGER_VECTOR is", r"  -- The predefined types for opening files")
wr("std/1993/standard.vhdl", s8)
t8 = t
t8, n = re.subn(
    r"^  (procedure|alias|function) "
    r"(JUSTIFY|TEE|SREAD|STRING_READ|BREAD|BINARY_READ|OREAD|OCTAL_READ|HREAD|HEX_READ|"
    r"SWRITE|STRING_WRITE|BWRITE|BINARY_WRITE|OWRITE|OCTAL_WRITE|HWRITE|HEX_WRITE)\b[^\n]*\n",
    "",
    t8,
    flags=re.M,
)
assert n == 24, n
t8 = drop_line(t8, r"  procedure WRITE \(L: inout LINE; VALUE: in REAL; FORMAT")
wr("std/1993/textio.vhdl", t8)

# ---- ieee 1993: NUMERIC_STD and NUMERIC_BIT from the 2008 text ------------------
# The IEEE 1076.3 (1995) texts carry a restrictive licence (#650), so the 1993 versions are
# derived from the Apache-2.0 text above: the types become the 1076.3 ones (arrays of
# STD_LOGIC / BIT, types of their own), what 2008 added is deleted, and a few small additions
# keep the 1076.3 function set. Every edit is listed in lib/SOURCES.md.
SUB = re.compile(r"^  (function|procedure)\b")
NUM_DROP = {  # names of subprograms 1076-2008 added (the scalar-operand forms are found by type)
    "MINIMUM", "MAXIMUM", "FIND_LEFTMOST", "FIND_RIGHTMOST", '"SLA"', '"SRA"', "TO_X01",
    "TO_X01Z", "TO_UX01", "IS_X", "TO_OSTRING", "TO_HSTRING", "READ", "WRITE", "OREAD",
    "OWRITE", "HREAD", "HWRITE",
}


def sub_end(lines, i):
    """Subprogram at lines[i]: (header text, index of its last line, is it a body)."""
    depth, text = 0, ""
    for k in range(i, len(lines)):
        code = lines[k].split("--")[0]
        text += code + "\n"
        for m in re.finditer(r"[();]|\bis\b", code, flags=re.I):
            c = m.group(0)
            depth += (c == "(") - (c == ")")
            if depth == 0 and c in (";", "is"):
                head = re.sub(r"\s+", " ", text[: text.lower().rfind(c)])
                if c == ";":
                    return head, k, False
                end = next(n for n in range(k, len(lines)) if lines[n].startswith("  end"))
                return head, end, True
    raise AssertionError(lines[i])


def prune(text, scalar, removed):
    """Drop every subprogram (with its Id line and comments) that 1076.3 does not have."""

    def denied(head, exported):
        h = head.upper()
        name = re.match(r"\s*(?:FUNCTION|PROCEDURE) (\"[^\"]+\"|\w+)", h).group(1)
        return (
            name.startswith('"?')
            or name in NUM_DROP
            or "SIZE_RES" in h
            # the exported forms with a scalar operand (a "-- Id:" line above); the local
            # helpers (ADD_UNSIGNED, ...) take a scalar carry bit and stay
            or (exported and name not in ("STD_MATCH", "TO_01") and re.search(rf"\b{scalar}\b", h) is not None)
        )

    lines, out, i = text.split("\n"), [], 0
    while i < len(lines):
        if SUB.match(lines[i]):
            head, end, is_body = sub_end(lines, i)
            stop = end + 1
            while not is_body and re.match(r"^\s*--", lines[stop]):
                stop += 1  # the comment lines after a declaration belong to it
            if denied(head, bool(out) and re.match(r"^\s*-- Id:", out[-1]) is not None):
                while re.match(r"^\s*-- Id:", out[-1]):
                    out.pop()
                if lines[stop].strip() == "":
                    stop += 1
                removed.append(head.strip())
                i = stop
                continue
        out.append(lines[i])
        i += 1
    return "\n".join(out)


def cut_tail(s, heading_re, end_re):
    """Delete from the banner line above the heading up to (excluding) the line matching end_re."""
    lines = s.split("\n")
    i = next(k for k, l in enumerate(lines) if re.search(heading_re, l)) - 1
    j = next(k for k in range(i, len(lines)) if re.match(end_re, lines[k]))
    return "\n".join(lines[:i] + lines[j:])


NUM_NOTE = """-- yavs: VHDL-1993 version, derived from the IEEE VASG 1076-2019 package source by
-- tools/derive_libs.py (see lib/SOURCES.md for the exact edits). UNSIGNED and SIGNED are
-- types of their own (arrays of {el}) as in IEEE 1076.3, and what 1076-2008 added is
-- deleted. The Apache License notice above applies.

"""
# 1076-2008 bodies call the predefined MINIMUM/MAXIMUM of INTEGER, which VHDL-1993 lacks.
NUM_MINMAX = """  -- yavs: MINIMUM and MAXIMUM of INTEGER are predefined from 1076-2008 only.
  function MAXIMUM (L, R : INTEGER) return INTEGER is
  begin
    if L > R then
      return L;
    end if;
    return R;
  end function MAXIMUM;

  function MINIMUM (L, R : INTEGER) return INTEGER is
  begin
    if L < R then
      return L;
    end if;
    return R;
  end function MINIMUM;

"""


# TO_01 of STD_ULOGIC_VECTOR is in 1076-2008 STD_LOGIC_1164 only (the algorithm is that one).
NUM_TO_01 = """  -- yavs: TO_01 of STD_ULOGIC_VECTOR is in the 1076-2008 STD_LOGIC_1164 only.
  function TO_01_VECTOR (S : STD_ULOGIC_VECTOR; XMAP : STD_ULOGIC)
    return STD_ULOGIC_VECTOR
  is
    variable RESULT      : STD_ULOGIC_VECTOR(S'length-1 downto 0);
    variable BAD_ELEMENT : BOOLEAN := false;
    alias XS             : STD_ULOGIC_VECTOR(S'length-1 downto 0) is S;
  begin
    for I in RESULT'range loop
      case XS(I) is
        when '0' | 'L' => RESULT(I)   := '0';
        when '1' | 'H' => RESULT(I)   := '1';
        when others    => BAD_ELEMENT := true;
      end case;
    end loop;
    if BAD_ELEMENT then
      for I in RESULT'range loop
        RESULT(I) := XMAP;
      end loop;
    end if;
    return RESULT;
  end function TO_01_VECTOR;

"""


def numeric93(pkg, scalar, el):
    """Return (declaration, body, removed subprogram headers) of NUMERIC_STD or NUMERIC_BIT."""
    d, b = rd(f"ieee/numeric_{pkg}.vhdl"), rd(f"ieee/numeric_{pkg}-body.vhdl")
    name, removed = f"NUMERIC_{pkg.upper()}", []
    d = cut_tail(d, r"-- string conversion and write operations", r"^end package")
    b = cut_tail(b, r"-- string conversion and write operations", r"^end package body")
    d, b = prune(d, scalar, removed), prune(b, scalar, removed)
    if pkg == "std":  # NUMERIC_BIT has the 1076.3 types already
        d, n = re.subn(
            r"  type UNRESOLVED_UNSIGNED is.*?subtype SIGNED is \(resolved\) UNRESOLVED_SIGNED;\n",
            f"  type UNSIGNED is array (NATURAL range <>) of {el};\n"
            f"  type SIGNED is array (NATURAL range <>) of {el};\n",
            d,
            flags=re.S,
        )
        assert n == 1
    d, b = d.replace("UNRESOLVED_", ""), b.replace("UNRESOLVED_", "")
    d = d.replace("use STD.TEXTIO.all;\n", "")
    note = NUM_NOTE.format(el=el)
    d = d.replace(f"package {name} is", note + f"package {name} is", 1)
    b = b.replace(f"package body {name} is", note + f"package body {name} is", 1)
    b = b.replace("  -- implementation controls\n", NUM_MINMAX + "  -- implementation controls\n", 1)
    if pkg == "std":
        b, n = re.subn(r"TO_01\(STD_ULOGIC_VECTOR\(S\), XMAP\)", "TO_01_VECTOR(STD_ULOGIC_VECTOR(S), XMAP)", b)
        assert n == 2
        b = b.replace("  -- implementation controls\n", NUM_TO_01 + "  -- implementation controls\n", 1)
        # 1076.3 has STD_MATCH for STD_LOGIC_VECTOR, a type of its own in 1993
        d, n = re.subn(
            r"(  -- Id: M.5\n)(  function STD_MATCH \(L, R : )STD_ULOGIC_VECTOR(\) return BOOLEAN;\n)",
            r"  -- Id: M.4\n\2STD_LOGIC_VECTOR\3  -- Result subtype: BOOLEAN\n"
            r"  -- Result: terms compared per STD_LOGIC_1164 intent\n\n\g<0>",
            d,
        )
        assert n == 1
        m5 = re.search(r"  -- Id: M.5\n.*?end function STD_MATCH;\n\n", b, flags=re.S).group(0)
        m4 = m5.replace("M.5", "M.4").replace("STD_ULOGIC_VECTOR", "STD_LOGIC_VECTOR", 1)
        b = b.replace(m5, m4.replace("STD_ULOGIC_VECTOR", "STD_LOGIC_VECTOR") + m5, 1)
    else:
        # RISING_EDGE and FALLING_EDGE of BIT are aliases of the 2008 STANDARD ones
        for fn, v in (("RISING_EDGE", "1"), ("FALLING_EDGE", "0")):
            d, n = re.subn(
                rf"  alias {fn} is STD.STANDARD.{fn}\n    \[STD.STANDARD.BIT return STD.STANDARD.BOOLEAN\];\n",
                f"  function {fn} (signal S : BIT) return BOOLEAN;\n",
                d,
            )
            assert n == 1
        edge = "".join(
            f"  -- Id: E.{k}\n  function {fn} (signal S : BIT) return BOOLEAN is\n  begin\n"
            f"    return S'EVENT and S = '{v}';\n  end function {fn};\n\n"
            for k, fn, v in ((1, "RISING_EDGE", "1"), (2, "FALLING_EDGE", "0"))
        )
        b = b.replace(f"end package body {name};", edge + f"end package body {name};", 1)
    return d, b, removed


for pkg, scalar, el in (("std", "STD_ULOGIC", "STD_LOGIC"), ("bit", "BIT", "BIT")):
    d93, b93, gone = numeric93(pkg, scalar, el)
    wr(f"ieee/1993/numeric_{pkg}.vhdl", d93)
    wr(f"ieee/1993/numeric_{pkg}-body.vhdl", b93)
    assert len(gone) == {"std": 224, "bit": 208}[pkg], len(gone)  # declarations plus bodies

# ---- ieee: identical files for both revisions + two tiny contexts --------------
CTX = """-- Copyright 2026 yavs authors. Licensed under the Apache License, Version 2.0
-- (http://www.apache.org/licenses/LICENSE-2.0). Content as specified by
-- IEEE Std 1076-2008 (original text, written for yavs).

context {n} is
  library IEEE;
{uses}end context {n};
"""
ieee_files = sorted(f for f in os.listdir(os.path.join(src, "ieee")) if f.endswith(".vhdl"))
for rev in ("2008", "2019"):
    for f in ieee_files:
        wr(f"ieee/{rev}/{f}", rd(f"ieee/{f}"))
    wr(
        f"ieee/{rev}/ieee_std_context.vhdl",
        CTX.format(
            n="IEEE_STD_CONTEXT",
            uses="  use IEEE.STD_LOGIC_1164.all;\n  use IEEE.NUMERIC_STD.all;\n",
        ),
    )
    wr(
        f"ieee/{rev}/ieee_bit_context.vhdl",
        CTX.format(n="IEEE_BIT_CONTEXT", uses="  use IEEE.NUMERIC_BIT.all;\n"),
    )

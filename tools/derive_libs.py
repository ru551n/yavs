"""Derive rsim's lib/std and lib/ieee from a checkout of vasg/Packages.

usage: derive_libs.py <packages-checkout> <outdir>

Everything is byte-for-byte upstream except the documented edits below.
Files are handled as latin-1 so the 8-bit bytes in STANDARD and NUMERIC_* survive.
"""
import os
import re
import sys

src, out = sys.argv[1:3]
NOTE = (
    "-- rsim: derived from the IEEE VASG 1076-2019 package source by\n"
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
    ("INTEGER", "-9223372036854775807 to 9223372036854775807"),
    ("REAL", "-1.7976931348623157e308 to 1.7976931348623157e308"),
    ("TIME", "-9223372036854775807 to 9223372036854775807"),
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
wr("std/2019/standard.vhdl", std19)
wr("std/2019/textio.vhdl", rd("std/textio.vhdl"))
wr("std/2019/env.vhdl", rd("std/env.vhdl"))
wr("std/2019/reflection.vhdl", rd("std/reflection.vhdl"))

# ---- std 2008: drop what 1076-2019 added ---------------------------------------
s = std19
s = drop_block(s, r"  type DIRECTION is \(", r"  type BOOLEAN is")
s = drop_line(s, r"    READ_WRITE_MODE,")
s = drop_block(s, r"  type FILE_OPEN_STATE is \(", r"  -- The 'FOREIGN attribute")
s = s.replace(  # 1076-2008 INTEGER is 32 bit
    "type INTEGER is range -9223372036854775807 to 9223372036854775807",
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

# ---- ieee: identical files for both revisions + two tiny contexts --------------
CTX = """-- Copyright 2026 rsim authors. Licensed under the Apache License, Version 2.0
-- (http://www.apache.org/licenses/LICENSE-2.0). Content as specified by
-- IEEE Std 1076-2008 (original text, written for rsim).

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

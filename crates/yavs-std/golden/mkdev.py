#!/usr/bin/env python3
"""Turn the golden test's failure list into sim1-deviations.txt entries.

Usage: cargo test -p yavs-std --test golden 2>&1 | grep -a -P '^(2008|2019)\\t' > fails.txt
       mkdev.py fails.txt >> sim1-deviations.txt
Every entry must be reviewed: the reason column says why yavs is right.
"""
import re
import sys

sys.path.insert(0, __import__("os").path.dirname(__file__))
from gen import esc  # noqa: E402

REASONS = [
    (r"^e1_(image|to_string)", "LRM 16.2.2: an extended identifier keeps its spelling; sim1 lower-cases it and undoubles backslashes"),
    (r"^read_(int|real|time)\t.*\tERROR$", "sim1 aborts inside READ; an unrepresentable value is good=false (LRM 16.4)"),
    (r"^read_real\t1\.5e\+", "an exponent needs digits (LRM 15.5.2); sim1 reads 1.5 and drops 'e+'"),
    (r"^read_time\t.*10000000 hr", "overflow is good=false; sim1 wraps the value"),
    (r"^read_time", "a physical literal may use a decimal literal (LRM 5.2.4.1, 15.5); sim1 READ(TIME) rejects it, sim2 accepts"),
    (r"^(real_digits|write_real|write_real_format)\t", "sim1 aborts on its fixed-size buffer (checked_sprintf); yavs prints the full value"),
    (r"^real_format\t1\.5\t(%5|%\.3)\t", "an incomplete conversion is an error; sim1 returns an empty string"),
    (r"^real_format\t.*\t(%[-+ #]|x=|%%|%-)", "C99 printf flags and literal text are supported; sim1 rejects them"),
    (r"^real_format", "sim1 aborts on its fixed-size buffer (checked_sprintf); yavs prints the full value"),
    (r"^write_time\t-9223372036854775808", "sim1 overflows negating TIME'LOW and prints garbage or aborts"),
    (r"^readline\ta\\x0db", "only CRs directly before the line end are dropped; sim1 deletes every CR in the line"),
    (r"^int_value\t(1__0|_1|1_)\t", "LRM 15.5.2: an underscore must stand between two digits"),
    (r"^int_value\t\+5", "a leading sign is accepted (sim2 agrees)"),
    (r"^int_value", "LRM 15.5: exponents and based literals are integer literals; sim1 'VALUE accepts decimal digits only"),
    (r"^real_value\t(\.5|5\.)\t", "LRM 15.5.2: a decimal literal needs digits on both sides of the point"),
    (r"^real_value", "LRM 15.5 literal syntax and IEEE rounding (sim2 agrees); sim1 'VALUE rejects based literals, underscores, extreme values"),
    (r"^time_value\t10000000 hr", "overflow is an error; sim1 wraps the value"),
    (r"^(time|dist)_value", "LRM 5.2.4.1/15.5: decimal, based and unit-only physical literals, rounded to the primary unit; extended unit names (sim2 agrees)"),
    (r"^e1_value\tFo o", "two words are not one literal; sim1 ignores the text after a space"),
    (r"^e1_value", "extended identifiers match exactly, with their case (LRM 15.4.3)"),
    (r"^char_value", "' ' is a character literal (sim2 agrees)"),
    (r"^trec_minus", "exact microsecond arithmetic; sim1's TIME_RECORD subtraction loses microseconds"),
    (r"^seconds_to_time", "a result outside TIME is an error; sim1 wraps"),
    (r"^(gmtime|epoch|trec)", "dates after 2038 are exact; sim1 wraps epochs beyond 2**31, and a year past 4095 is outside TIME_RECORD"),
]

rows = {}
order = []
for line in open(sys.argv[1], encoding="latin-1"):
    line = line.rstrip("\n")
    m = re.match(r"^(2008|2019)\t(.*)\t(\[.*\]|ERROR \(.*\))\t(expected .*)$", line)
    if not m:
        sys.exit("cannot parse: " + line)
    rev, head, got, exp = m.groups()
    out = "[" + esc(got[1:-1].encode("latin-1")) + "]" if got.startswith("[") else "ERROR"
    key = (head, out, exp)
    if key not in rows:
        order.append(key)
        rows[key] = set()
    rows[key].add(rev)

def c_printf(head):
    """The C (Python %) result for the real formatting cases, or None."""
    a = head.split("\t")
    f, args = a[0], a[1:]
    if f == "real_format":
        return args[1] % float(args[0])
    if f == "real_digits":
        d = int(args[1])
        return ("%.17g" if d == 0 else "%%.%df" % d) % float(args[0])
    if f == "write_real":
        x, side, field, d = float(args[0]), args[1], int(args[2]), int(args[3])
        s = ("%e" if d == 0 else "%%.%df" % d) % x
        return s.rjust(field) if side == "right" else s.ljust(field)
    if f == "write_real_format":
        return args[1] % float(args[0])
    return None


for head, out, exp in order:
    try:
        ref = c_printf(head)
    except (ValueError, TypeError):
        ref = "ERROR"
    if ref == "ERROR" and ref != "ERROR":
        sys.exit(f"C printf rejects {head} but yavs gives {out}")
    if ref is not None and out != "[" + ref + "]" and ref != "ERROR":
        sys.exit(f"C printf disagrees for {head}: yavs {out}, C [{ref}]")
    revs = rows[(head, out, exp)]
    rev = "*" if len(revs) == 2 else revs.pop()
    probe = head + "\t" + ("ERROR" if exp == "expected ERROR" else out)
    reason = next((r for p, r in REASONS if re.search(p, probe) or re.search(p, head)), None)
    if reason is None:
        sys.exit("no reason for: " + probe)
    print(f"{rev}\t{head}\t{out}\t{reason}")

#!/usr/bin/env python3
"""A stand-in for the Verilog side of `yavs analyze`, for testing the Verilog test tools before the analyzer exists.

It reads the command line of the planned analyzer (`--work=LIB:DIR`, `-I DIR`, `-D NAME[=VAL]`, files), knows only the
preprocessor-stage diagnostics E4001 to E4005 and W4006, and reports them as `--message-format json` lines in the shape of
`yavs-diag`. It is not a Verilog parser. Delete it when `yavs analyze *.v` exists (tools/verilog/AGENTS.md).

FAKE_YAVS_BUG=double-e4001|miss-e4001|plain-error makes it wrong on purpose, so the tools can be seen to fail.
"""
import json, os, re, sys
from pathlib import Path

BUG = os.environ.get("FAKE_YAVS_BUG", "")
SV_WORDS = re.compile(r"\b(always_ff|always_comb|always_latch|typedef)\b")
CHECKS = re.compile(r"\$(setuphold|setup|hold|recovery|removal|recrem|skew|period|width|nochange)\b")


def mask(t):
    """Blank comments and string contents in one pass, keeping every offset and newline."""
    def blank(m):
        s = m.group(0)
        return '"' + re.sub(r"[^\n]", " ", s[1:-1]) + '"' if s[0] == '"' else re.sub(r"[^\n]", " ", s)
    return re.sub(r'//[^\n]*|/\*.*?\*/|"(?:\\.|[^"\\\n])*"', blank, t, flags=re.S)


class Run:
    def __init__(self, incs, defs, fmt):
        self.incs, self.defs, self.fmt, self.errors, self.timing_warned = incs, set(defs), fmt, 0, False

    def report(self, level, code, msg, file, line, col):
        if level == "error":
            self.errors += 1
        if self.fmt == "json":
            span = {"file": file, "byte_start": 0, "byte_end": 0, "line_start": line, "column_start": col,
                    "line_end": line, "column_end": col + 1, "primary": True, "label": msg}
            print(json.dumps({"level": level, "code": code, "message": msg, "spans": [span], "children": [], "rendered": msg}), file=sys.stderr)
        else:
            print(f"{level}[{code}]: {msg}\n --> {file}:{line}:{col}", file=sys.stderr)

    def scan(self, path, shown):
        if path.suffix in (".sv", ".svh"):
            self.report("error", "E4002", "SystemVerilog source file; only IEEE 1364-2005 is supported", shown, 1, 1)
            return
        raw = path.read_text(errors="replace")
        orig = raw.split("\n")
        stack, active, seen_ts = [], True, False
        for ln, line in enumerate(mask(raw).split("\n"), 1):
            for m in re.finditer(r"`(\w+)(?:[ \t]+([^\n]*))?", line):
                d, arg, col = m.group(1), (m.group(2) or "").strip(), m.start() + 1
                if d in ("ifdef", "ifndef"):
                    name = arg.split()[0] if arg else ""
                    stack.append((active, ln, col, (name in self.defs) == (d == "ifdef"), False))
                    active = active and stack[-1][3]
                elif d in ("elsif", "else", "endif"):
                    if not stack:
                        self.report("error", "E4004", f"`{d} without `ifdef", shown, ln, col)
                        return
                    outer, l0, c0, cond, taken = stack.pop()
                    if d == "endif":
                        active = outer
                    else:
                        name = arg.split()[0] if arg else ""
                        take = (not (taken or cond)) and (d == "else" or name in self.defs)
                        stack.append((outer, l0, c0, take, taken or cond))
                        active = outer and take
                elif not active:
                    continue
                elif d == "define":
                    self.defs.add(arg.split("(")[0].split()[0])
                elif d == "undef":
                    self.defs.discard(arg)
                elif d == "timescale":
                    seen_ts = True
                elif d == "include":
                    inc = re.match(r'"([^"]+)"', orig[ln - 1][m.start(2):].strip()) if m.group(2) else None
                    found = next((p for p in [path.parent / inc.group(1)] + [Path(i) / inc.group(1) for i in self.incs] if p.is_file()), None) if inc else None
                    if found is None:
                        self.report("error", "E4005", f"include file {inc.group(1) if inc else arg} not found; searched: {', '.join(self.incs) or 'no -I directories'}", shown, ln, col)
                        return
                    n = self.errors
                    self.scan(found, str(found))
                    if self.errors > n:
                        return
                elif d == "pragma" and re.match(r"protect\s+begin_protected\b", arg) or d in ("protect", "protected"):
                    if BUG != "miss-e4001":
                        self.report("error", "E4001", "encrypted (IEEE 1735 protected) module body", shown, ln, col)
                    if BUG == "double-e4001":
                        self.report("error", "E4001", "encrypted (IEEE 1735 protected) module body", shown, ln, col)
                    return
                elif d == "pragma" and re.match(r"protect\s+(begin|end)\b", arg):
                    self.report("warning", "W4007", "plaintext protect marker ignored", shown, ln, col)
            if active:
                if re.match(r"\s*(module|macromodule|primitive)\b", line):
                    if not seen_ts:
                        self.report("warning", "W4006", "no `timescale: using 1ns/1ps", shown, ln, 1)
                        seen_ts = True
                m = SV_WORDS.search(line)
                if m:
                    self.report("error", "E4003", f"SystemVerilog keyword `{m.group(1)}` in a Verilog file", shown, ln, m.start() + 1)
                    return
                if CHECKS.search(line) and "XIL_TIMING" in self.defs and not self.timing_warned:
                    self.timing_warned = True
                    self.report("warning", "W4008", "timing checks are not evaluated", shown, ln, 1)
                if BUG == "plain-error" and "assign" in line:
                    self.report("error", "E4003", "injected false positive", shown, ln, 1)
                    return
        if stack:
            _, l0, c0, _, _ = stack[-1]
            self.report("error", "E4004", "`ifdef without `endif", shown, l0, c0)


def main(argv):
    fmt, incs, defs, files, i = "human", [], [], [], 0
    while i < len(argv):
        a = argv[i]
        if a == "--message-format":
            fmt, i = argv[i + 1], i + 1
        elif a in ("-I", "-D"):
            (incs if a == "-I" else defs).append(argv[i + 1].split("=")[0] if a == "-D" else argv[i + 1])
            i += 1
        elif a.startswith("--work=") or a in ("analyze", "--no-cache"):
            if a.startswith("--work="):
                Path(a.split(":", 1)[1]).mkdir(parents=True, exist_ok=True)
        elif a.startswith("-"):
            sys.exit(f"fake_yavs: unknown option {a}")
        else:
            files.append(a)
        i += 1
    run = Run(incs, defs, fmt)
    for f in files:
        run.scan(Path(f), f)
        if run.errors:  # like the real command: the first failing file stops the batch
            break
    return 1 if run.errors else 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))

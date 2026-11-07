"""Shared helpers of the Verilog test tools: run `yavs analyze` and read its JSON diagnostics."""
import json, os, shutil, subprocess, sys
from pathlib import Path

REPO = Path(__file__).resolve().parents[2]
WRAP = ["timeout", "300", "systemd-run", "--user", "--scope", "-q", "-p", "MemoryMax=2G"]


def die(prog, msg):
    print(f"{prog}: {msg}", file=sys.stderr)
    sys.exit(2)


def find_yavs(arg):
    """The analyzer command as an argv list: --yavs, else $YAVS, else target/release/yavs, else yavs on PATH."""
    import shlex
    if arg or os.environ.get("YAVS"):
        return shlex.split(arg or os.environ["YAVS"])
    for p in (REPO / "target/release/yavs", REPO / "target/debug/yavs"):
        if p.exists():
            return [str(p)]
    return [shutil.which("yavs") or "yavs"]


def analyze(yavs, work, libname, args, files, cwd, wrap=True):
    """One `yavs analyze` call: (exit status, diagnostics). A diagnostic is (level, code, file, line, col)."""
    argv = yavs + ["--message-format", "json", "analyze", f"--work={libname}:{work}"] + args + [str(f) for f in files]
    try:
        p = subprocess.run(argv if not wrap else WRAP + argv, capture_output=True, text=True, errors="replace", cwd=cwd)
    except OSError as e:
        raise SystemExit(f"cannot run {argv[0]}: {e}")
    return p.returncode, parse(p.stdout + "\n" + p.stderr), p.stdout + p.stderr


def parse(text):
    out = []
    for ln in text.splitlines():
        if not ln.startswith("{"):
            continue
        try:
            d = json.loads(ln)
        except ValueError:
            continue
        sp = next((s for s in d.get("spans", []) if s.get("primary")), None)
        out.append((d["level"], d["code"], sp["file"] if sp else "", sp["line_start"] if sp else 0, sp["column_start"] if sp else 0))
    return out

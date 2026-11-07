#!/usr/bin/env python3
"""Analysis corpus gate of the Verilog front end: `yavs analyze` over real cell-model and IP libraries.

  tools/verilog/corpus.py vendor|cells [list|gen|check|bless] [--root DIR] [--yavs CMD] [-j N] [--only CAT,..]
                                       [--manifest FILE] [--no-wrap]

  vendor  the AMD/Xilinx 2026.1 install, read in place and never copied or committed (local only; skipped with a notice when the
          install is absent). Root: --root, $YAVS_VENDOR_DATA, else <root>/2026.1/data under ~/Xilinx, /opt/Xilinx, /tools/Xilinx
          (the directory that holds verilog/src/glbl.v). Categories: unisims, retarget, unimacro, unifast, glbl in both views
          (default and -D XIL_TIMING), unisims-sv, ip-plain and ip-encrypted (ip/**/*.v), protected (*.vp).
  cells   the yosys techlibs simulation models (ISC, fetched by tools/fetch-corpus.sh into the corpus cache; CI-visible).

  list    enumerate and classify, print counts and findings; write nothing
  gen     write the manifest from the classification (what the Verilog plan expects), without running yavs
  check   run yavs and compare every file with the manifest (default); exit 0 as expected, 1 differences, 2 usage error
  bless   write the manifest from this run (then review the diff)

The manifest tools/verilog/<source>.expected has one line per file: `OUTCOME LIB VIEW PATH [-I DIR]...`, paths relative to the
root, in a fixed order. OUTCOME is PASS (no error) or the sorted error codes, with `xN` when E4001 occurs N times (so E4001 is
exactly one E4001; EXIT<n> is a nonzero exit without an error diagnostic). Warnings are counted, not compared. The unisims, retarget,
unimacro, unifast and glbl files of one library and view go in one call (as an integration analyses a library); if that call
fails, the files are repeated one by one and a failure of the session alone is a DIFF. IP and protected files are one call each,
with the include directories the manifest lists.
"""
import argparse, collections, concurrent.futures as cf, os, re, shlex, sys, tempfile
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
import vcommon  # noqa: E402

HERE = Path(__file__).resolve().parent
VIEWS = {"default": [], "timing": ["-D", "XIL_TIMING"]}
SV_STRONG = re.compile(r"\b(?:always_ff|always_comb|always_latch|typedef|package|endpackage|endinterface|endclass|struct)\b|\benum\s*(?:\{|logic|bit|int)\b|\bimport\s+\w+\s*::|\b(?:int|longint|shortint|string)\s+[A-Za-z_]\w*\s*[;=,)\[]")
ENVELOPE = re.compile(r"^[ \t]*`(?:pragma[ \t]+protect[ \t]+begin_protected|protect|protected)\b", re.M)
# other tools' form: the pragma sits in a line comment (//pragma protect begin_protected); the Verilog plan names only the directive forms
COMMENT_ENVELOPE = re.compile(r"^[ \t]*//[ \t]*pragma[ \t]+protect[ \t]+begin_protected\b", re.M)
DIRECTIVES = set("""define undef ifdef ifndef elsif else endif include timescale default_nettype celldefine endcelldefine resetall
line pragma protect endprotect protected endprotected unconnected_drive nounconnected_drive begin_keywords end_keywords
uselib suppress_faults nosuppress_faults enable_portfaults disable_portfaults delay_mode_path delay_mode_unit delay_mode_zero
delay_mode_distributed accelerate noaccelerate expand_vectornets noexpand_vectornets remove_gatenames noremove_gatenames
remove_netnames noremove_netnames unconnected_drive""".split())


LEX = re.compile(r'//[^\n]*|/\*.*?\*/|"(?:\\.|[^"\\\n])*"', re.S)


def strip(t, strings=False):
    """Comments removed and, unless strings=True, strings emptied; one pass (`//*****` is a line comment, not a block one)."""
    return LEX.sub(lambda m: m.group(0) if m.group(0)[0] == '"' and strings else '""' if m.group(0)[0] == '"' else " ", t)


def rd(p):
    return p.read_text(errors="replace")


def classify(text):
    """The outcome the Verilog plan expects of a .v file: E4001 (envelope), E4003 (SystemVerilog syntax) or PASS."""
    t = strip(text)
    return "E4001" if ENVELOPE.search(t) or COMMENT_ENVELOPE.search(text) else "E4003" if SV_STRONG.search(t) else "PASS"


# ---- enumeration: entries are dicts {cat, lib, view, path (relative to root), inc: [relative dirs], want, batch}

def vendor_root(arg):
    cands = [arg, os.environ.get("YAVS_VENDOR_DATA")]
    cands += [Path(b).expanduser() / "2026.1/data" for b in ("~/Xilinx", "/opt/Xilinx", "/tools/Xilinx")]
    # ponytail: the first root with the exact glbl.v is taken; two 2026.1 installs on one host need --root
    return next((Path(c).expanduser().resolve() for c in cands if c and (Path(c).expanduser() / "verilog/src/glbl.v").is_file()), None)


def cells_root(arg):
    cands = [arg, os.environ.get("YAVS_CORPUS"), "~/.cache/rsim-corpus", "~/.cache/rsim-corpora"]
    return next((Path(c).expanduser() / "yosys" for c in cands if c and (Path(c).expanduser() / "yosys/techlibs").is_dir()), None)


def include_dirs(root, f, index, text, seen, findings):
    """-I directories for f: its own, and the directory of every include (transitively), found by name when not beside the includer."""
    dirs = [f.parent]
    for inc in re.findall(r'^[ \t]*`include[ \t]+"([^"]+)"', strip(text, True), re.M):
        local = (f.parent / inc).resolve()
        if local.is_file():
            hit = local
        else:
            # only a plain file name can be found by name: -I DIR cannot make `../x.vh` or `sub/x.vh` resolve from elsewhere
            cand = sorted(index.get(inc, []), key=lambda d: -len(os.path.commonprefix([str(d), str(f.parent)])))
            if not cand:
                findings["unresolved include"].append(f"{f.relative_to(root)}: {inc}")
                continue
            if len({d for d in cand}) > 1:
                findings["ambiguous include"].append(f"{f.relative_to(root)}: {inc} ({len(cand)} places, nearest used)")
            hit = cand[0] / Path(inc).name
        if hit not in seen:
            seen.add(hit)
            dirs += include_dirs(root, hit, index, rd(hit), seen, findings)
            dirs.append(hit.parent)
    return list(dict.fromkeys(dirs))


def macro_suspects(f, text, index):
    """Macros used but defined in neither the file nor what it includes (textual: ignores `ifdef, so a hint only)."""
    defined, todo, seen = set(), [(f, text)], set()
    while todo:
        p, t = todo.pop()
        t = strip(t, True)
        defined |= set(re.findall(r"`define[ \t]+(\w+)", t))
        for inc in re.findall(r'`include[ \t]+"([^"]+)"', t):
            q = (p.parent / inc) if (p.parent / inc).is_file() else next((d / Path(inc).name for d in index.get(Path(inc).name, [])), None)
            if q and q not in seen and q.is_file():
                seen.add(q)
                todo.append((q, rd(q)))
    used = set(re.findall(r"`(\w+)", strip(text)))
    return sorted(used - defined - DIRECTIVES)


# One copy of a memory model per simulator product, named after it; the product names stay out of tracked files (root AGENTS.md
# section 2a), so these directories are counted and not gated.
SKIP_DIRS = ("ip/xilinx/ddr4_v2_2/data/dlib/ultrascale/ddr4_sdram/tb/ddr4_model/samsung", "ip/xilinx/hbm_v1_0/verif/model",
             "ip/xilinx/noc_hbm_responder_v1_0/hdl/memory_model", "ip/xilinx/mig_7series_v4_2/data/dlib/7series/qdriiplus_sram/sim_")  # a prefix: the sibling directories of the tool variants


def enum_vendor(root):
    src, ents, findings = root / "verilog/src", [], collections.defaultdict(list)

    index, files = collections.defaultdict(list), []

    def add(cat, lib, view, p, want, inc=(), batch=False):
        ents.append(dict(cat=cat, lib=lib, view=view, path=str(p.relative_to(root)), inc=[str(Path(d).relative_to(root)) for d in inc], want=want, batch=batch))

    for view in VIEWS:
        add("glbl", "work", view, src / "glbl.v", "PASS", batch=True)
    for cat, lib in (("unisims", "unisims_ver"), ("retarget", "unisims_ver"), ("unimacro", "unimacro_ver"), ("unifast", "unifast_ver")):
        for view in VIEWS:
            for p in sorted((src / cat).glob("*.v")):
                add(cat, lib, view, p, "PASS", batch=True)
    for p in sorted((src / "unisims").glob("*.sv")):
        add("unisims-sv", "work", "default", p, "E4002")
    for r, _, fs in os.walk(root / "ip"):
        for n in fs:
            if n.endswith((".v", ".vh", ".svh", ".h")):
                index[n].append(Path(r))
            if n.endswith(".v"):
                files.append(Path(r) / n)
    skipped = [p for p in files if p.relative_to(root).as_posix().startswith(SKIP_DIRS)]
    files = [p for p in files if p not in skipped]
    prot = [src / "unisim_retarget_comp.vp"] + sorted((root / "secureip").rglob("*.vp")) + sorted((root / "ip").rglob("*.vp"))
    skipped += [p for p in prot if p.relative_to(root).as_posix().startswith(SKIP_DIRS)]
    prot = [p for p in prot if p not in skipped]
    for p in prot:
        want = classify(rd(p))
        inc = include_dirs(root, p, index, rd(p), {p}, findings)
        if COMMENT_ENVELOPE.search(rd(p)):
            findings["envelope only in a comment (//pragma protect)"].append(str(p.relative_to(root)))
        if want == "PASS":
            findings["protected file with no envelope"].append(str(p.relative_to(root)))
        add("protected", "work", "default", p, want, inc)
    for p in sorted(files):
        t = rd(p)
        want = classify(t)
        cat = "ip-encrypted" if want == "E4001" else "ip-plain"
        seen = {p}
        inc = [] if want == "E4001" else include_dirs(root, p, index, t, seen, findings)
        if want == "PASS" and any(classify(rd(q)) == "E4001" for q in seen - {p}):
            want, cat = "E4001", "ip-includes-encrypted"  # ponytail: textual closure, so an include in inactive code predicts wrong; bless fixes it
            findings["plaintext file that includes an encrypted file (E4001 predicted)"].append(str(p.relative_to(root)))
        add(cat, "ip", "default", p, want, inc)
        if cat != "ip-encrypted":
            if not re.search(r"^\s*(?:module|macromodule|primitive)\b", strip(t), re.M):
                findings["include fragment (no module)"].append(str(p.relative_to(root)))
            if (m := macro_suspects(p, t, index)):
                findings["macro with no definition in sight"].append(f"{p.relative_to(root)}: {' '.join(m[:6])}")
            if want == "E4003":
                findings["SystemVerilog syntax in .v (E4003)"].append(str(p.relative_to(root)))
    findings["not gated"].append(f"tool-specific memory-model copies: {len(skipped)}")
    for pat in ("*.svp", "*.sv", "*.svh"):
        findings["not gated"].append(f"{pat}: " + str(sum(1 for r, _, fs in os.walk(root) for n in fs if Path(n).match(pat))))
    return ents, findings


def enum_cells(root):
    ents, findings = [], collections.defaultdict(list)
    index = collections.defaultdict(list)
    for p in root.rglob("*.vh"):
        index[p.name].append(p.parent)
    for p in sorted(root.glob("techlibs/**/*sim*.v")):
        t = rd(p)
        ents.append(dict(cat="cells", lib="work", view="default", path=str(p.relative_to(root)), want=classify(t) if classify(t) != "E4001" else "PASS",
                         inc=[str(d.relative_to(root)) for d in include_dirs(root, p, index, t, {p}, findings)], batch=False))
        if SV_STRONG.search(strip(t)):
            findings["SystemVerilog syntax (E4003 predicted)"].append(str(p.relative_to(root)))
    return ents, findings


SOURCES = {"vendor": (vendor_root, enum_vendor, "the AMD/Xilinx 2026.1 install was not found (this gate is local only); set $YAVS_VENDOR_DATA to <install>/2026.1/data or pass --root"),
           "cells": (cells_root, enum_cells, "the yosys cell models were not found; run tools/fetch-corpus.sh (or pass --root with the corpus cache)")}


# ---- manifest

def line(e, want):
    return shlex.join([want, e["lib"], e["view"], e["path"]] + [x for d in e["inc"] for x in ("-I", d)])


def read_manifest(path):
    ents = []
    for n, ln in enumerate(path.read_text().splitlines(), 1):
        if not ln.strip() or ln.startswith("#"):
            continue
        t = shlex.split(ln)
        inc = [t[i + 1] for i in range(4, len(t), 2)]
        ents.append(dict(want=t[0], lib=t[1], view=t[2], path=t[3], inc=inc, cat=None, batch=False))
    return ents


def write_manifest(path, name, ents, wants):
    head = f"# tools/verilog/corpus.py {name}: expected outcome per file, `OUTCOME LIB VIEW PATH [-I DIR]...`; paths only, no vendor content.\n"
    path.write_text(head + "".join(line(e, w) + "\n" for e, w in zip(ents, wants)))


def outcome(errs, rc):
    if not errs:
        return "PASS" if rc == 0 else f"EXIT{rc}"
    c = collections.Counter(errs)
    return ",".join(k + (f"x{v}" if v > 1 and k == "E4001" else "") for k, v in sorted(c.items()))  # only the envelope count is exact


# ---- running

def run_group(a, yavs, root, tmp, tag, ents):
    """Analyze the entries of one call; returns ({entry index: outcome}, warnings, text of a failed session or '')."""
    e0 = ents[0][1]
    args = VIEWS[e0["view"]] + [x for d in e0["inc"] for x in ("-I", str(root / d))]
    files = [root / e["path"] for _, e in ents]
    rc, diags, text = vcommon.analyze(yavs, os.path.join(tmp, tag), e0["lib"], args, files, root, not a.no_wrap)
    by = collections.defaultdict(list)
    for lvl, code, f, *_ in diags:  # a one-file call owns every diagnostic, also those in files it includes
        by[str(root / e0["path"]) if len(ents) == 1 else os.path.realpath(root / f)].append((lvl, code))
    res, warn = {}, 0
    for i, e in ents:
        ds = by.pop(str(root / e["path"]) if len(ents) == 1 else os.path.realpath(root / e["path"]), [])
        res[i] = outcome([c for l, c in ds if l == "error" or c == "E4001"], rc if len(ents) == 1 else 0)  # E4001 counts at any severity
        warn += sum(l == "warning" for l, _ in ds)
    stray = [f"{f}: {c}" for f, ds in by.items() for l, c in ds if l == "error"]
    return res, warn, text if rc and len(ents) > 1 else "", stray


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("source", choices=SOURCES)
    ap.add_argument("action", nargs="?", default="check", choices=["list", "gen", "check", "bless"])
    ap.add_argument("--root")
    ap.add_argument("--yavs")
    ap.add_argument("--manifest", type=Path)
    ap.add_argument("--only", help="comma-separated categories")
    ap.add_argument("-j", type=int, default=4)
    ap.add_argument("--no-wrap", action="store_true")
    a = ap.parse_args()
    locate, enum, hint = SOURCES[a.source]
    root = locate(a.root)
    if root is None:
        print(f"{a.source}: skipped, {hint}")
        return 0
    manifest = a.manifest or HERE / f"{a.source}.expected"
    ents, findings = enum(root)
    cats = collections.Counter(e["cat"] + ("/" + e["view"] if e["cat"] in ("unisims", "retarget", "unimacro", "unifast", "glbl") else "") for e in ents)
    print(f"{a.source}: root {root}")
    for c, n in sorted(cats.items()):
        print(f"  {c:<22}{n:>6}")
    print(f"  {'total':<22}{len(ents):>6}")
    for k, v in findings.items():
        print(f"  {k}: {len(v)}" if k != "not gated" else f"  not gated: {'; '.join(v)}")
        if a.action == "list" and k != "not gated":
            for x in v[:40]:
                print("    " + x)
    if a.action == "list":
        return 0
    if a.action == "gen":
        write_manifest(manifest, a.source, ents, [e["want"] for e in ents])
        print(f"wrote {manifest}")
        return 0

    exp = read_manifest(manifest) if manifest.exists() else None
    if a.action == "check" and exp is None:
        vcommon.die("corpus.py", f"{manifest} missing; create it with `gen` or `bless`")
    bad = []
    if exp is not None and a.action == "check":  # the manifest must list exactly what the install holds
        if [(e["lib"], e["view"], e["path"], e["inc"]) for e in exp] != [(e["lib"], e["view"], e["path"], e["inc"]) for e in ents]:
            bad.append(f"the manifest lists other files than the install holds ({len(exp)} against {len(ents)}); run `gen` and review the diff")
    only = set(a.only.split(",")) if a.only else None
    sel = [(i, e) for i, e in enumerate(ents) if not only or e["cat"] in only]
    groups = collections.defaultdict(list)
    for i, e in sel:
        groups[(e["lib"], e["view"]) if e["batch"] else i].append((i, e))
    yavs = vcommon.find_yavs(a.yavs)
    got, warns = {}, 0
    with tempfile.TemporaryDirectory(prefix="yavs-verilog-corpus-") as tmp:
        os.environ["YAVS_STD_CACHE"] = os.path.join(tmp, "std")

        def work(item):
            k, g = item
            tag = f"g{abs(hash(k))}"
            res, w, failed, stray = run_group(a, yavs, root, tmp, tag, g)
            if failed:  # a call stops at its first failing file: repeat file by file for the exact result of each
                res, w2 = {}, 0
                for i, e in g:
                    r, w1, _, s = run_group(a, yavs, root, tmp, f"{tag}-{i}", [(i, e)])
                    res.update(r)
                    w2 += w1
                    stray += s
                if all(v == "PASS" for v in res.values()):
                    stray.append(f"the call of {len(g)} {g[0][1]['lib']} files failed but every file passes alone: {failed.strip().splitlines()[0][:200]}")
                w = w2
            return res, w, stray

        with cf.ThreadPoolExecutor(a.j) as pool:
            for res, w, stray in pool.map(work, groups.items()):
                got.update(res)
                warns += w
                bad += [f"unattributed error: {s}" for s in stray]
    for i, e in sel:
        want = exp[i]["want"] if exp is not None and a.action == "check" and len(exp) == len(ents) else e["want"]
        if got[i] != want:
            bad.append(f"{e['path']} ({e['lib']}, {e['view']}): got {got[i]}, expected {want}")
    if a.action == "bless":
        if only:
            vcommon.die("corpus.py", "bless needs the complete run (no --only)")
        write_manifest(manifest, a.source, ents, [got[i] for i in range(len(ents))])
        print(f"wrote {manifest}")
        bad = [b for b in bad if b.startswith("unattributed")]
    out = collections.Counter(got.values())
    print(f"{len(sel)} files, {warns} warnings; outcomes: " + ", ".join(f"{k} {v}" for k, v in sorted(out.items())))
    for b in bad[:60]:
        print("DIFF:", b)
    if len(bad) > 60:
        print(f"... and {len(bad) - 60} more differences")
    return 1 if bad else 0


if __name__ == "__main__":
    sys.exit(main())

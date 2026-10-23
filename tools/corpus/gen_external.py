"""External library corpora: specs, candidate lists, and validation with the reference tools.

Used by gen_manifests.py (imported). A spec names a source (see SOURCES in gen_manifests.py), the files
that make up the corpus (glob -> library, first match wins), skip rules with reasons, and optionally
test benches. The candidate list is ordered with the regex dependency scan (gen_manifests.order_files).

Validation (`gen_manifests.py --validate NAME...`): every candidate is analyzed in order, one file per call, by
sim1 and by sim2 (2008) or sim1 and sim3 (2019). A file the tools reject is dropped with the first error line as
reason, the chain is re-run until it is clean, and the drops are recorded in rejects/<name>.json (which the
generator reads, so the checked-in manifests are reproducible). Where sim1 and sim2 disagree, sim3 decides (1993 has no
third reference: the file is dropped).
The commands come from tools/sims.local.conf (keys gen_cmd, gen_setup, gen_licence; see tools/AGENTS.md).
"""
import fnmatch, json, os, re, shlex, shutil, subprocess, sys
from concurrent.futures import ThreadPoolExecutor
from pathlib import Path

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE.parent))
import simconf  # noqa: E402
REJECTS = HERE / "rejects"  # one json per manifest name
WRAP = ["timeout", "300", "systemd-run", "--user", "--scope", "-q", "-p", "MemoryMax=2G"]
# A reference that needs a licence (key gen_licence = 1) gets it from the caller's environment
# (LM_LICENSE_FILE, SALT_LICENSE_FILE or SALT_LICENSE_SERVER), passed through; some insist on
# SALT_LICENSE_SERVER, so licence_env() fills that from the other two when it is unset. Values are
# never printed or written down.
LICENCE_FAIL = re.compile(r"Unable to checkout|Unable to find the license|Invalid license environment|lmgrd|FlexNet", re.I)
WORK = Path("~/.cache/rsim-corpus2").expanduser()
STD2 = {"1993": "93c", "2008": "08", "2019": "19"}


def licence_env():
    env = dict(os.environ)
    if "SALT_LICENSE_SERVER" not in env:
        for v in ("LM_LICENSE_FILE", "SALT_LICENSE_FILE"):
            if env.get(v):
                env["SALT_LICENSE_SERVER"] = env[v]
                break
    return env


def run_tool(tool, std, lib, wd, path, flags=()):
    """Analyze one file with the reference `tool` (a label); key gen_cmd is an argv template with
    {std} {std2} {lib} {dir} {file} and {flags} (as a word of its own)."""
    wd = Path(wd)
    d = wd / tool
    d.mkdir(parents=True, exist_ok=True)
    cmd = simconf.get(tool, "gen_cmd")
    if cmd is None:
        sys.exit(f"{tool}: key gen_cmd not configured (tools/sims.local.conf)")
    env = licence_env() if simconf.get(tool, "gen_licence") else None
    setup = simconf.get(tool, "gen_setup")  # once per library, in the tool's directory (library creation and mapping)
    if setup and not (d / lib).exists():
        subprocess.run(["bash", "-c", setup], cwd=d, env={**(env or os.environ), "LIB": lib, "LIBDIR": str(d / lib)}, capture_output=True)
    argv = []
    for w in shlex.split(cmd):
        if w == "{flags}":
            argv += list(flags)
        else:
            argv.append(w.format(std=std, std2=STD2[std], lib=lib, dir=d, file=path))
    if env is not None and not shutil.which(argv[0]):
        sys.exit(f"{tool}: {argv[0]} not found on PATH")
    p = subprocess.run(WRAP + argv, capture_output=True, text=True, errors="replace", cwd=d if env is not None else None, env=env)
    out = p.stdout + p.stderr
    if env is not None and p.returncode and LICENCE_FAIL.search(out):
        sys.exit(f"{tool}: failed to get a licence; check LM_LICENSE_FILE, SALT_LICENSE_FILE or "
                 "SALT_LICENSE_SERVER in the calling environment")
    return p.returncode, out


def first_error(text):
    for ln in text.splitlines():
        if re.search(r"\*\* Error|error|Error:|Fatal", ln) and not ln.lstrip().startswith("-->"):
            ln = re.sub(r"\x1b\[[0-9;]*m", "", ln).strip()
            return re.sub(r"/\S*/([^/\s]+\.vhdl?)", r"\1", ln)[:200]
    return (text.strip().splitlines() or ["no output"])[0][:200]


def chain(tool, std, files, base, wd, flags):
    """Analyze files in order; returns {index: (ok, text)}."""
    shutil.rmtree(Path(wd) / tool, ignore_errors=True)
    out = {}
    for i, (lib, src, path, _) in enumerate(files):
        rc, text = run_tool(tool, std, lib, wd, base[src] / path, flags)
        out[i] = (rc == 0, text)
    return out


def validate(name, std, files, base, flags):
    """Returns (accepted files, {path: reason}, log lines)."""
    second = "sim2" if std in ("1993", "2008") else "sim3"
    rej, log = {}, []
    cur = list(files)
    for it in range(12):
        wd, log = WORK / f"val-{name}", []
        with ThreadPoolExecutor(2) as ex:
            fa = ex.submit(chain, "sim1", std, cur, base, wd, flags.get("sim1", []))
            fb = ex.submit(chain, second, std, cur, base, wd, flags.get(second, []))
            a, b = fa.result(), fb.result()
        drop, third = [], None
        for i, f in enumerate(cur):
            (oa, ta), (ob, tb) = a[i], b[i]
            if oa and ob:
                continue
            why = []
            if not oa:
                why.append("sim1: " + first_error(ta))
            if not ob:
                why.append(f"{second}: " + first_error(tb))
            why = "; ".join(why)
            if std == "2019":  # sim1 is the reference; sim3 lacks many 2019 features, so it can only add acceptances
                ok = oa or ob
                verdict = "" if ok else "rejected by sim1 and sim3: "
                if ok:
                    log.append(f"kept {f[2]}: sim1={'ok' if oa else 'FAIL'} sim3={'ok' if ob else 'FAIL'} ({why})")
            elif oa == ob:
                ok, verdict = False, "rejected by the reference tools: "
            elif std == "1993":  # no third reference at 1993: a file the two disagree on is dropped
                ok = False
                verdict = f"sim1 and sim2 disagree (sim1={'ok' if oa else 'FAIL'}, sim2={'ok' if ob else 'FAIL'}), no third reference at 1993: "
                log.append(f"disagree {f[2]}: sim1={'ok' if oa else 'FAIL'} sim2={'ok' if ob else 'FAIL'} ({why})")
            else:  # sim1 and sim2 disagree: sim3 decides
                if third is None:
                    third = chain("sim3", std, cur, base, wd, flags.get("sim3", []))
                ok = third[cur.index(f)][0]
                verdict = f"sim1 and sim2 disagree (sim1={'ok' if oa else 'FAIL'}, sim2={'ok' if ob else 'FAIL'}), sim3 rejects: "
                log.append(f"disagree {f[2]}: sim1={'ok' if oa else 'FAIL'} sim2={'ok' if ob else 'FAIL'} sim3={'ok' if ok else 'FAIL'} ({why})")
            if not ok:
                drop.append((i, f, f"round {it + 1}: " + verdict + why))
        if not drop:
            break
        for i, f, why in drop:
            rej[f[2]] = why
        gone = {f[2] for _, f, _ in drop}
        cur = [f for f in cur if f[2] not in gone]
        print(f"{name}: iteration {it + 1}: dropped {len(drop)}, {len(cur)} left", file=sys.stderr, flush=True)
    shutil.rmtree(WORK / f"val-{name}", ignore_errors=True)
    return cur, rej, log


def load_rejects():
    return {p.stem: json.loads(p.read_text()) for p in REJECTS.glob("*.json")}


def save_rejects(name, rej, log):
    REJECTS.mkdir(exist_ok=True)
    (REJECTS / f"{name}.json").write_text(json.dumps({"rejects": rej, "log": log}, indent=1, sort_keys=True) + "\n")


# ---------------------------------------------------------------- spec -> candidate list
def candidates(spec, root, std):
    """[(lib, rel path)] for the files of the spec (src then tb), and [(rel, reason)] skipped by rule."""
    files, skipped, seen = [], [], set()
    for kind in ("src", "tb"):
        for glob, lib in spec.get(kind, []):
            for p in sorted(root.glob(glob)):
                rel = str(p.relative_to(root))
                if not p.is_file() or rel in seen or p.suffix.lower() not in (".vhd", ".vhdl"):
                    continue
                seen.add(rel)
                why = next((w for g, w in spec.get("skip", []) if fnmatch.fnmatch(rel, g)), None)
                if why:
                    skipped.append((rel, why))
                else:
                    files.append((lib, rel, kind))
    return files, skipped


ENT = re.compile(r"^\s*entity\s+(\w+)\s+is\b", re.I | re.M)


def tb_entries(files, base, spec):
    out = []
    tbs = {f[2] for f in spec.get("_tbfiles", [])}
    for lib, src, path, _ in files:
        if path in tbs:
            t = Path(base[src] / path).read_text(encoding="latin-1")
            vu = bool(re.search(r"\brunner_cfg\b|\btest_runner_cfg\b", t, re.I))
            for e in ENT.findall(t):
                out.append({"lib": lib, "path": path, "top": e, "vunit": vu})
    return out


# ---------------------------------------------------------------- generator glue
def gen_ext(name, std, G):
    """G is the gen_manifests module (find_root, order_files, GEN, SOURCES). Returns the GEN tuple plus an extras dict."""
    spec = SPECS[name]
    base, files, tbfiles, excl = {}, [], [], []
    for sid, (url, commit, lic, note) in spec["sources"].items():
        G.SOURCES[sid] = (url, commit, note, lic)
        base[sid] = G.find_root(sid)
    seen = set()
    if "lister" in spec:  # the project's own scripts give the order: no glob, no regex scan
        sid = next(iter(spec["sources"]))
        files = [(lib, sid, rel, base[sid] / rel) for lib, rel in spec["lister"](base[sid])]
    for kind in ("src", "tb"):
        for sid, glob, lib, *only in spec.get(kind, []):
            if only and std not in only[0]:
                continue
            for p in sorted(base[sid].glob(glob)):
                rel = str(p.relative_to(base[sid]))
                if not p.is_file() or (sid, rel) in seen or p.suffix.lower() not in (".vhd", ".vhdl"):
                    continue
                seen.add((sid, rel))
                why = next((w for s2, g, w in spec.get("skip", []) if s2 == sid and fnmatch.fnmatch(rel, g)), None)
                if why:
                    excl.append((sid, rel, why))
                    continue
                files.append((lib, sid, rel, base[sid] / rel))
                if kind == "tb":
                    tbfiles.append(rel)
    ordered = files if "lister" in spec else G.order_files(files)
    rej = {} if G.VALIDATING else load_rejects().get(f"{name}-{std}", {}).get("rejects", {})
    kept = []
    for lib, sid, rel, ab in ordered:
        if rel in rej:
            excl.append((sid, rel, rej[rel]))
        else:
            kept.append((lib, sid, rel, std))
    pre, psrc = [], []
    for pn in spec.get("prefix", []):
        s, f, _, _, *_ = G.GEN[pn](std)
        pre += [x for x in f if x[2] not in rej]
        psrc += [x for x in s if x not in psrc]
    for x in psrc:
        base[x] = G.find_root(x)
    allf = pre + kept
    srcs = list(dict.fromkeys(psrc + list(spec["sources"])))
    tbs = []
    if tbfiles:
        for lib, sid, rel, _ in kept:
            if rel in tbfiles:
                t = (base[sid] / rel).read_text(encoding="latin-1")
                vu = bool(re.search(r"\brunner_cfg\b", t, re.I))
                for e in ENT.findall(t):
                    tbs.append({"lib": lib, "source": sid, "path": rel, "top": e, "vunit": vu})
    ex = {"testbenches": dict(spec.get("tbinfo", {}), bench=tbs) if tbs else None, "base": base,
          "spec": spec, "prefix_n": len(pre)}
    return srcs, allf, excl, spec.get(f"note-{std}", spec["note"]).replace("STD", std), ex


def grlib_files(root):
    """[(lib, path)] in GRLIB's own compile order for the leon3mp template with inferred technology, without the design's own files.
    GRLIB's script generator (bin/scriptgen, Tcl, writes only into its working directory) runs in a temporary copy of the template
    directory; the target named by key grlib_target of sim1 (tools/sims.local.conf) is a make target whose file lists every source in
    order, one command per file. This reads that list (library from --work, only .vhd, only files under lib/)."""
    import tempfile
    target = simconf.get("sim1", "grlib_target")
    if not target:
        sys.exit("sim1: key grlib_target not configured (tools/sims.local.conf): the GRLIB make target that writes the ordered file list")
    with tempfile.TemporaryDirectory(prefix="grlib-gen-", dir=WORK.parent) as td:
        d = Path(td) / "leon3mp"
        shutil.copytree(root / "designs/leon3mp", d)
        p = subprocess.run(["make", "-s", f"GRLIB={root}", "TECHLIBS=inferred", target], cwd=d, capture_output=True, text=True)
        if p.returncode:
            sys.exit(f"grlib: make {target} failed (needs make and tclsh):\n{p.stderr[-800:]}")
        out = []
        for ln in (d / target).read_text().splitlines():
            m = re.search(r"--work=(\w+)\S* .* (\S+\.vhd)\s*$", ln)
            if m and m.group(2).startswith(f"{root}/lib/"):
                out.append((m.group(1), str(Path(m.group(2)).relative_to(root))))
        return out


VU_RUN = ("VUnit run.py (project's own); each test bench is run with its generics from the project runner, "
          "test_runner_setup/cleanup from vunit_lib")
PLAIN_PASS = "no assertion of severity error or worse fires and the simulation ends by itself (all stimulus consumed)"
SPECS = {
    "en_cl_fix": dict(
        stds=("2008",), prefix=["vunit"],
        note="order: regex dependency scan; library per the project's sim/run.py; VHDL STD",
        sources={"en_cl_fix": ("https://github.com/open-logic/en_cl_fix", "ecd7dd026837997f2cd8bc4861769abc6cbbc73c", "MIT",
                               "Enclustra en_cl_fix as pinned by open-logic (submodule 3rdParty/en_cl_fix)")},
        src=[("en_cl_fix", "hdl/*.vhd", "lib_en_cl_fix_tb")],
        tb=[("en_cl_fix", "tb/*.vhd", "lib_en_cl_fix_tb"), ("en_cl_fix", "tb/util/*.vhd", "lib_en_cl_fix_tb")],
        skip=[],
        tbinfo=dict(runner="VUnit sim/run.py (needs the en_tb submodule and a co-simulation with the Python model for some benches)",
                    passes="VUnit test_runner_cleanup with no failed check; benches check results against values computed in the bench")),

    "open-logic": dict(
        stds=("2008",), prefix=["vunit"],
        flags={"sim1": ["--relaxed"], "sim2": ["-frelaxed-rules"], "yavs": ["--relaxed"]},
        note="order: regex dependency scan; libraries olo (src + en_cl_fix) and olo_tb as in sim/run.py; VHDL STD",
        sources={"open-logic": ("https://github.com/open-logic/open-logic", "d9f123aae1c6022288782668067311f9794da0a2",
                                "LGPL-2.1 with the PSI HDL library FPGA exception", "main, 2026-10-13"),
                 "en_cl_fix": ("https://github.com/open-logic/en_cl_fix", "ecd7dd026837997f2cd8bc4861769abc6cbbc73c", "MIT",
                               "submodule 3rdParty/en_cl_fix, compiled into library olo")},
        src=[("open-logic", "src/**/*.vhd", "olo"), ("en_cl_fix", "hdl/*.vhd", "olo")],
        tb=[("open-logic", "test/tb/legacy/*.vhd", "olo_tb"), ("open-logic", "test/**/*.vhd", "olo_tb")],
        tbinfo=dict(runner="VUnit sim/run.py (--sim1/--sim2); every bench is run once per generic configuration listed in the area's config (test/*/config.py style add_configs)",
                    passes="VUnit test_runner_cleanup reached with no failed check/assert; each bench ends by itself")),
    "vhdl-compliance": dict(
        stds=("2008", "2019"), prefix=["vunit"],
        note="order: regex dependency scan; library vhdl_2008 / vhdl_2019 as in run.py; VHDL STD",
        sources={"vhdl-compliance": ("https://github.com/VHDL/Compliance-Tests", "7c4449ee5027d933ee3d7c0465bd0a5d06f9c05e", "Apache-2.0",
                                     "VHDL-2008 and VHDL-2019 feature tests, one self-contained VUnit test bench per language feature")},
        src=[],
        tb=[("vhdl-compliance", "vhdl_2008/*.vhd", "vhdl_2008"), ("vhdl-compliance", "vhdl_2019/*.vhd", "vhdl_2019", ("2019",))],
        tbinfo=dict(runner="run.py of the project (VUnit; uses sim1/sim4/other simulators with 2019, others with 2008); each bench is one test case",
                    passes="VUnit test case passes (test_runner_cleanup, no error()). Stub benches for unimplemented LCS issues call error() on purpose and are expected to FAIL when run")),
    "vhdl-interfaces": dict(
        stds=("2019",),
        note="order: compileorder.list of the project (VHDLRevision: VHDL-2019, VHDLLibrary: Interfaces); VHDL STD",
        sources={"vhdl-interfaces": ("https://github.com/VHDL/Interfaces", "d2fd277fdf3b4606b9bc335751077b6303ad683f", "Apache-2.0",
                                     "VHDL-2019 interface (view/mode view) declarations for AMBA, Avalon, Wishbone, IO")},
        src=[("vhdl-interfaces", "**/*.vhdl", "Interfaces")], tb=[]),
    "vhdl-tool-checks": dict(
        stds=("2008", "2019"),
        note="order: single file per standard; VHDL STD",
        sources={"vhdl-tool-checks": ("https://github.com/Paebbels/VHDL-Tool-Checks", "08cf0cc8238da587502fd96d525b4c992867ae76", "Apache-2.0",
                                      "tool feature probes for range handling")},
        src=[("vhdl-tool-checks", "Ranges.2008.vhdl", "work", ("2008",)), ("vhdl-tool-checks", "Ranges.2019.vhdl", "work", ("2019",))],
        tb=[]),
    "json-for-vhdl": dict(
        stds=("2008",), prefix=["vunit"],
        note="order: regex dependency scan; library JSON as in regression.tcl; VHDL STD",
        sources={"json-for-vhdl": ("https://github.com/Paebbels/JSON-for-VHDL", "2ab1ebc2c788ececce152a49ffda706f84570e95", "Apache-2.0",
                                   "JSON parser in VHDL-2008 (protected types, string handling, file reading)")},
        src=[("json-for-vhdl", "src/*.vhdl", "JSON")],
        tb=[("json-for-vhdl", "examples/*.vhdl", "JSON"), ("json-for-vhdl", "tests/JSONTestSuite/*.vhdl", "JSON")],
        tbinfo=dict(runner="tests/VUnit/run.py (Boards_VUnit, Encodings_VUnit); Boards0/1/2 and TopLevel are plain benches run by name (regression.tcl, the sim2 script)",
                    passes="VUnit benches: test_runner_cleanup with no failed check; plain benches: no assertion failure, simulation ends by itself")),
    "neorv32": dict(
        stds=("2008",),
        note="order: regex dependency scan; library neorv32 as in the sim2 script; VHDL STD",
        sources={"neorv32": ("https://github.com/stnolting/neorv32", "b2620454ebb2407e81a6a1965ef52e2a860b0432", "BSD-3-Clause",
                             "sparse: /rtl/ and /sim/ only (software, docs, other setups are not needed)")},
        src=[("neorv32", "rtl/core/*.vhd", "neorv32"), ("neorv32", "rtl/system_integration/*.vhd", "neorv32"),
             ("neorv32", "rtl/test_setups/*.vhd", "neorv32")],
        tb=[("neorv32", "sim/*.vhd", "neorv32")],
        tbinfo=dict(runner="the sim2 script: sim2 -m neorv32_tb; sim2 -r neorv32_tb --stop-time=10ms --ieee-asserts=disable --assert-level=error (boot ROM runs a program; UART output is the result)",
                    passes="no assertion of severity error or worse within 10 ms of simulated time; the bench prints the UART output of the running program, the default image is the bootloader")),
    "grlib": dict(
        stds=("2008", "1993"), lister=grlib_files,
        flags={"sim1": ["--relaxed"], "sim2": ["-frelaxed-rules", "-fsynopsys"], "yavs": ["--relaxed"]},
        **{"flags-1993": {"sim1": ["--relaxed"], "sim2": ["-frelaxed-rules", "-fsynopsys", "-fexplicit"], "yavs": ["--relaxed"]},
           "note-1993": "order: GRLIB's script generator for designs/leon3mp (TECHLIBS=inferred, its LIBSKIP/DIRSKIP), library per file as it names it; VHDL STD, the standard of GRLIB's own files (its scripts pass the 93 flags)",
           "mdnote-1993": "GRLIB is VHDL-93 code; this is the manifest at VHDL-1993, where `std_logic_vector` is a type of its own and the paired overloads of `stdio` and `testlib` are legal. The sim2 flags are GRLIB's own (`-fexplicit`, the synopsys library switch)."},
        note="order: GRLIB's script generator for designs/leon3mp (TECHLIBS=inferred, its LIBSKIP/DIRSKIP), library per file as it names it; VHDL STD, which GRLIB itself does not use (its files are VHDL-93 and its scripts pass the 93 flags)",
        sources={"grlib": ("tools/fetch-corpus.sh (source grlib)", "020f83888d67e140bdca248f51fcc783fd8f99a0", "GPL-2.0-only",
                           "GRLIB 2026.3 build 4301 (Gaisler IP library, LEON3 and NOEL-V); sparse: lib/, bin/, the leon3mp, leon3-minimal and noelv-generic templates")},
        mdnote="GRLIB is VHDL-93 code (its scripts pass the 93 flags to every tool; `stdio.vhd` and `testlib.vhd` are marked `vhdlstd=93` in its file lists). Under 2008 `std_logic_vector` is a subtype of `std_ulogic_vector`, so the paired overloads of those two packages are homographs and every reference rejects them; the files that use them (the processor core, the bus controller, the simulation models) drop out as cascades. What is listed is the rest.",
        tbinfo=dict(runner="designs/leon3mp (testbench.vhd, own files not in this list): the template's own simulation targets",
                    passes="the self test in ram.srec prints 'Test passed' on the UART and halts the processor, which the bench reports as a failure assertion")),
    "vests": dict(
        stds=("1993",),
        note="order: regex dependency scan; library work, every file of the compliant directories; VHDL STD",
        sources={"vests": ("https://github.com/nickg/vests", "f23a8cd103dd00ba58c1de434aea72a29efcd514", "GPL-2.0-only",
                           "VHDL Experimental Suite of Tests (University of Cincinnati) for 1076-1993: the compliant tests of the billowitch, ashenden and clifton-labs groups")},
        src=[("vests", "vhdl-93/billowitch/compliant/*.vhd", "work"), ("vests", "vhdl-93/ashenden/compliant/*.vhd", "work"),
             ("vests", "vhdl-93/clifton-labs/compliant/*.vhd", "work")],
        tb=[],
        mdnote="The tests are small self-checking benches written for the 1993 standard; the non-compliant and disputed directories are not part of this list. Many are standalone, the Ashenden ones share packages and entities."),
    "neotrng": dict(
        stds=("2008",),
        note="order: regex dependency scan; VHDL STD",
        sources={"neotrng": ("https://github.com/stnolting/neoTRNG", "c688149f70fbd582eac1029cbfec4fcf97b719b2", "BSD-3-Clause",
                             "true random number generator, single entity + bench")},
        src=[("neotrng", "rtl/*.vhd", "neotrng")], tb=[("neotrng", "sim/*.vhd", "neotrng")],
        tbinfo=dict(runner="sim/ (sim2, sim2 -r neotrng_tb --stop-time=...)", passes="runs without assertion failure; the bench prints random words (no golden output)")),
}


SPECS.update({
    "poc": dict(
        stds=("2008",),
        note="order: regex dependency scan; library poc for src/ plus the generic board config (my_config_GENERIC, CI my_project), test for tb/; VHDL STD",
        sources={"poc": ("https://github.com/VLSI-EDA/PoC", "8c39b2407a97ec81b8601c4b64e4d2e2141ab9cf", "Apache-2.0",
                         "Chair of VLSI-Design (TU Dresden) Pile of Cores, archived; the project's own simulation package PoC.simulation")},
        src=[("poc", "src/**/*.vhdl", "poc"), ("poc", "tb/common/my_config_GENERIC.vhdl", "poc"), ("poc", "tools/GitLab-CI/my_project.vhdl", "poc")],
        tb=[("poc", "tb/**/*.vhdl", "test")],
        skip=[("poc", "src/xil/*", "Xilinx primitives (unisim)"), ("poc", "src/alt/*", "Altera/Intel primitives"),
              ("poc", "*.v93.vhdl", "VHDL-93 variant of a unit that also has a 2008 file"),
              ("poc", "tb/common/my_config_[!G]*", "board configuration other than GENERIC"),
              ("poc", "tb/common/config_tb.vhdl", "needs a board configuration package selected at run time")],
        tbinfo=dict(runner="poc.sh sim2|sim3 <tb entity> (PoC's own tool chain; *.files list each bench's dependencies); equivalently sim2 -r <tb>",
                    passes="PoC.simulation: the bench calls simAssert/simFinish and prints 'SIMULATION RESULT = PASSED'; severity failure on a failed assertion")),
    "satcat5": dict(
        stds=("2008",),
        note="order: regex dependency scan; everything in library work as in the project's Vivado/sim3 scripts; VHDL STD",
        sources={"satcat5": ("https://github.com/the-aerospace-corporation/satcat5", "58402dcbe64adc9c30d00c08edc20e3aef314ac6", "CERN-OHL-W-2.0",
                             "Aerospace Corporation SatCat5 Ethernet switch/FPGA library")},
        src=[("satcat5", "src/vhdl/common/*.vhd", "work")],
        tb=[("satcat5", "sim/vhdl/*.vhd", "work")],
        tbinfo=dict(runner="plain VHDL benches (*_tb), one entity each, run by the project's sim/ scripts for sim3/Vivado; any simulator with --stop-time works",
                    passes="no 'assert ... severity error' fires (benches use assert with severity error and finish by running out of stimulus)")),
    "vhdl-extras": dict(
        stds=("2008",),
        note="order: regex dependency scan; libraries extras / extras_2008 / test per directory as in the makefile (sim3 -work basename dir); VHDL STD",
        sources={"vhdl-extras": ("https://github.com/kevinpt/vhdl-extras", "b8916c4942e99d02fac04e25799cb2c996eb5870", "MIT",
                                 "license stated in README.rst and the file headers (no LICENSE file)")},
        src=[("vhdl-extras", "rtl/extras/*.vhdl", "extras"), ("vhdl-extras", "rtl/extras_2008/*.vhdl", "extras_2008")],
        tb=[("vhdl-extras", "rtl/test/*.vhdl", "test"), ("vhdl-extras", "rtl/test_2008/*.vhdl", "test_2008")],
        tbinfo=dict(runner="makefile + scripts/ (sim3, sim3 -c <entity> -do 'run -all; quit'); each test_* entity stands alone",
                    passes="the bench ends by itself; failures are 'assert ... severity error/failure' (the project scripts grep the log for '** Error' / '** Failure')")),
    "vhlib": dict(
        stds=("2008",),
        note="order: regex dependency scan; library work as vhdeps does it; VHDL STD",
        sources={"vhlib": ("https://github.com/abs-tudelft/vhlib", "b69bbdaf7dacc06c861dd0982bb6653482912a76", "Apache-2.0",
                           "vendor agnostic VHDL-2008 stream/util library with its own test case framework (sim/TestCase_pkg)")},
        src=[("vhlib", "util/*.vhd", "work"), ("vhlib", "stream/*.vhd", "work"), ("vhlib", "stream/model/*.vhd", "work")],
        tb=[("vhlib", "sim/*.vhd", "work"), ("vhlib", "stream/test/**/*.vhd", "work")],
        tbinfo=dict(runner="vhdeps sim2|sim3 <top> (files *.sim.08.vhd are simulation-only VHDL-2008; tops are the *_tb entities, test cases *_tc are compiled into them)",
                    passes="TestCase_pkg: every subtest ends with tc_pass; the bench reports the summary and stops, 'Test case failed' (severity failure) otherwise")),
    "fletcher": dict(
        stds=("2008",),
        note="order: regex dependency scan; library work for fletcher and its vhlib submodule (as the project's compile scripts); VHDL STD",
        sources={"fletcher": ("https://github.com/abs-tudelft/fletcher", "45be6d6cb2c9546cee151c657dc675e03b11e302", "Apache-2.0",
                              "Apache Arrow to FPGA hardware components"),
                 "vhlib": ("https://github.com/abs-tudelft/vhlib", "b69bbdaf7dacc06c861dd0982bb6653482912a76", "Apache-2.0",
                           "submodule hardware/vhlib")},
        src=[("vhlib", "util/*.vhd", "work"), ("vhlib", "stream/*.vhd", "work"), ("vhlib", "stream/model/*.vhd", "work"),
             ("vhlib", "sim/*.vhd", "work"), ("fletcher", "hardware/*/*.vhd", "work")],
        tb=[("fletcher", "hardware/**/test/**/*.vhd", "work")],
        skip=[("fletcher", "hardware/vhlib/*", "submodule, taken from source vhlib")],
        tbinfo=dict(runner="plain benches (*_tb) run through vhdeps (sim2/sim3) in the project's CI",
                    passes="no 'severity failure' assertion; benches stop when their stimulus is exhausted")),
    "vhdl-ideas": dict(
        stds=("2019",),
        note="order: regex dependency scan; library work; VHDL STD",
        sources={"vhdl-ideas": ("https://github.com/bpadalino/vhdl-ideas", "f5970bd9f66bf12179f651ab43bfa0db946a7035", "MIT",
                                "personal collection of VHDL-2008/2019 utilities (generic packages, interfaces, BFMs)")},
        src=[("vhdl-ideas", "**/*.vhd", "work"), ("vhdl-ideas", "**/*.vhdl", "work")], tb=[]),
    "surf": dict(
        stds=("2008",),
        note="order: regex dependency scan; every file in library surf (ruckus puts them there); VHDL STD",
        sources={"surf": ("https://github.com/slaclab/surf", "f0b5e3a7b5ab191be1cb5f4453e418299181c9d8", "BSD-3-Clause-style (SLAC)",
                          "SLAC SURF firmware library; sparse checkout of *.vhd/*.vhdl only")},
        src=[("surf", "**/*.vhd", "surf"), ("surf", "**/*.vhdl", "surf")],
        skip=[("surf", "*/tb/*", "test benches are listed separately"), ("surf", "simlink/*", "needs the SLAC simlink C library (VHPI)")],
        tb=[],
        ),
})
SPECS["surf"]["tb"] = [("surf", "**/tb/*.vhd", "surf")]
SPECS["surf"]["skip"] = [("surf", "simlink/*", "needs the SLAC simlink C library (VHPI)")]
SPECS["surf"]["tbinfo"] = dict(runner="plain benches run with sim2/sim3 by the project (ruckus based); no runner script in the repository subset",
                               passes="benches end by themselves; failures are assertions of severity failure")
_vt = "vunit/vhdl"
SPECS["vunit-tests"] = dict(
    stds=("2008", "2019"), prefix=["vunit"],
    note="order: regex dependency scan; one library per VUnit module test directory (tb_<module>_lib) and per example (lib as in its run.py); VHDL STD",
    sources={"vunit": ("https://github.com/VUnit/vunit", "d627259f9795f28f3d1f57a3a3de9f9978bd49b8", "MPL-2.0",
                       "sparse: /vunit/vhdl/ and /examples/vhdl/")},
    src=[],
    tb=[("vunit", f"{_vt}/{m}/test/*.vhd", f"tb_{m}_lib") for m in
        ("check", "com", "data_types", "dictionary", "logging", "path", "random", "run", "string_ops", "verification_components")]
       + [("vunit", f"examples/vhdl/{e}/*.vhd", "lib") for e in
          ("array", "array_axis_vcs", "axi_dma", "check", "com", "composite_generics", "coverage", "generate_tests",
           "logging", "osvvm_log_integration", "run", "third_party_integration", "uart", "user_guide", "vhdl_configuration")]
       + [("vunit", f"examples/vhdl/{e}/src/*.vhd", "lib") for e in ("axi_dma", "uart")]
       + [("vunit", f"examples/vhdl/{e}/src/test/*.vhd", "lib") for e in ("axi_dma", "uart")],
    skip=[("vunit", "*/test/*-93*", "VHDL-93 variant")],
    tbinfo=dict(runner="the module's run.py / examples/vhdl/*/run.py with VUnit itself (python3 run.py --sim1|--sim2); VUnit's own tests are in vunit/vhdl/*/test",
                passes="every test case reports pass: VUnit test_runner_cleanup with no failed check. tb_*_failed style benches are written to expect failures via mock/unmock and pass overall"))


def neutral(s):
    """Documents name no other projects: tool names in generated prose become sim1/sim2/sim3."""
    for a, b in (("sim1", "sim1"), ("sim1", "sim1"), ("sim2", "sim2"), ("sim2", "sim2"), ("sim3", "sim3"), ("sim3", "sim3")):
        s = re.sub(rf"(?<![\w/.-]){a}(?![\w/-])", b, s)
    return re.sub(r"\s*\(Using LLVM [\d.]+\)", "", re.sub(r"^sim1 ", "Reference simulator sim1 ", s))


def notes(name, std, nfiles, nprefix):
    """Write <name>.<suffix>.md from the manifest numbers and the recorded rejects (suffix: key corpus_suffix of sim1)."""
    import collections, subprocess as sp
    spec = SPECS[name.rsplit("-", 1)[0]]
    suffix = simconf.get("sim1", "corpus_suffix", "sim1")
    ver = sp.run(["bash", "-c", simconf.get("sim1", "gen_version", "echo sim1")], capture_output=True, text=True).stdout.splitlines()[0]
    d = load_rejects().get(name, {"rejects": {}, "log": []})
    rej = d["rejects"]
    ne = HERE / f"{name}.{suffix}.expected"
    nacc = sum(1 for x in ne.read_text().splitlines() if x.startswith("PASS")) if ne.exists() else nfiles
    L = [f"# {name}: reference simulator validation", "", neutral(re.sub(r"^\S+ ", "sim1 ", ver)), "",
         "sim1 is the primary reference simulator, sim2 the second and sim3 the third; the roles are fixed by the corpus tooling.", "",
         f"Every manifest entry analyzed in order into `~/.cache/rsim-corpora/sim1-{name}` with `tools/corpus/run.py {name} --tool sim1 --bless` "
         f"(one file per call, `--std={std}`, each call under `timeout 300 systemd-run --user --scope -q -p MemoryMax=2G`).", "",
         f"- Files: {nfiles}; accepted by sim1: {nacc}; rejected by sim1: {nfiles - nacc}. The manifest lists only files the reference tools accept "
         f"({'sim1 and sim2, sim3 as tiebreaker' if std == '2008' else 'sim1 and sim2; a file they disagree on is dropped' if std == '1993' else 'sim1, sim3 as second witness'}); the rest are under Excluded.",
         f"- Flags needed: {', '.join(spec.get('flags', {}).get('sim1', [])) or 'none'}."]
    if nprefix:
        L.append(f"- The first {nprefix} entries are the {' + '.join(spec['prefix'])}-{std} library files the test benches need (the test runner library and a verification library); the project's own files follow.")
    src = spec["sources"]
    L.append("- Licenses: " + "; ".join(f"{k}: {v[2]}" for k, v in src.items()) + ".")
    if spec.get(f"mdnote-{std}", spec.get("mdnote")):
        L += ["", spec.get(f"mdnote-{std}", spec.get("mdnote"))]
    if rej:
        first = [(k, v) for k, v in rej.items() if v.startswith("round 1:")]
        casc = len(rej) - len(first)
        L += ["", f"## Excluded by validation ({len(rej)} files; {len(first)} direct, {casc} cascades of a missing unit in a later round)", ""]
        groups = collections.defaultdict(list)
        for k, v in first:
            key = re.sub(r"[A-Za-z_0-9.\-]+\.vhdl?(:\d+(:\d+)?)?:?", "", v.split(": ", 1)[1])
            key = re.sub(r"\"[^\"]*\"", "\"_\"", key)[:150]
            if re.search(r"not found in library|library \S+ not found|resource library", key):
                key = "uses a design unit or library that is missing: its own file is excluded or it is a vendor library (cascade)"
            groups[key].append(k)
        for key, ks in sorted(groups.items(), key=lambda kv: -len(kv[1])):
            L.append(f"- {len(ks)} x {neutral(key.strip())} (e.g. `{ks[0]}`)")
    if ne.exists() and nacc < nfiles:
        L += ["", "## Kept although sim1 rejects them (sim2 and sim3 accept)", ""] + \
             [f"- `{x.split(None, 2)[2]}`" for x in ne.read_text().splitlines() if x.startswith("FAIL")]
    if d["log"]:
        L += ["", "## Tool disagreements", ""] + [f"- {neutral(x)}" for x in d["log"][:40]] + ([f"- ... {len(d['log']) - 40} more in rejects/{name}.json"] if len(d["log"]) > 40 else [])
    (HERE / f"{name}.{suffix}.md").write_text("\n".join(L) + "\n")

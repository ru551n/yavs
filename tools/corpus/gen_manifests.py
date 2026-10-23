#!/usr/bin/env python3
"""Generate tools/corpus/<name>.toml compile manifests from each project's own build description.

    gen_manifests.py [--root DIR ...] [NAME ...]     (names: vunit-2008 osvvm-2019 uvvm-2008 yavs-lib-2019 ...)

Sources are read from the corpus cache (see tools/fetch-corpus.sh) and never copied.
  vunit : vunit/builtins.py rules (file-name standard tags, per-library globs) + a regex
          dependency scan for the order (VUnit also scans; it has no static order list)
  osvvm : OsvvmLibraries.pro, interpreted by a small Tcl subset with the tool settings of sim1 (keys osvvm_tool_name, osvvm_tool_vendor of tools/sims.local.conf)
  uvvm  : script/component_list.txt + each component's script/compile_order.txt
  yavs-lib : lib/ORDER.md
"""
import argparse, glob, os, re, sys
from pathlib import Path
import gen_external
sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
import simconf
from pathlib import Path

HERE = Path(__file__).resolve().parent
REPO = HERE.parent.parent
ROOTS = [os.environ.get("YAVS_CORPUS", ""), "~/.cache/rsim-corpus", "~/.cache/rsim-corpora"]

SOURCES = {
    "vunit": ("https://github.com/VUnit/vunit", "d627259f9795f28f3d1f57a3a3de9f9978bd49b8",
              "sparse: /vunit/vhdl/ (its osvvm submodule is source 'osvvm' below)"),
    "osvvm": ("https://github.com/OSVVM/OSVVM", "2104430dbbd2c7c1fff2ee02e1996944c59b62c2",
              "VUnit's bundled OSVVM copy (VUnit submodule vunit/vhdl/osvvm, 2023.04)"),
    "osvvm-libraries": ("https://github.com/OSVVM/OsvvmLibraries", "ab47a2637008a0709bbf593aa1ab0060dcc39aac",
                        "tag 2026.09, with submodules (osvvm, Common, UART, AXI4, ...)"),
    "hdl-modules": ("https://github.com/hdl-modules/hdl-modules", "c4f01c4bb69aab66c879f545e80abb477f1ebf35",
                    "release v6.2.1"),
    "uvvm": ("https://github.com/UVVM/UVVM", "90d56e93c542bf0d5e2ab9f791cc7395bd1aa896", "read-only shallow clone"),
}


def find_root(sid):
    for r in ROOTS:
        if r and (Path(r).expanduser() / sid).is_dir():
            return Path(r).expanduser() / sid
    sys.exit(f"source {sid} not found; run tools/fetch-corpus.sh")


# ---------------------------------------------------------------- VUnit
STDS = ["93", "2002", "2008", "2019"]


def vunit_std_ok(base, std):
    """vunit/builtins.py _add_files: NNNNp = NNNN and later, NNNNm = NNNN and earlier, NNNN = only."""
    ok = set()
    for i, s in enumerate(STDS):
        if s + "p" in base:
            ok.update(STDS[i:])
        elif s + "m" in base:
            ok.update(STDS[: i + 1])
        elif s in base:
            ok.add(s)
    return not ok or std in ok


def strip_comments(t):
    t = re.sub(r"/\*.*?\*/", " ", t, flags=re.S)
    return re.sub(r"--[^\n]*", "", t)


UNIT_RES = [
    ("entity", re.compile(r"^\s*entity\s+(\w+)\s+is\b", re.I | re.M)),
    ("package", re.compile(r"^\s*package\s+(?!body\b)(\w+)\s+is\b", re.I | re.M)),
    ("context", re.compile(r"^\s*context\s+(\w+)\s+is\b", re.I | re.M)),
]
ARCH = re.compile(r"^\s*architecture\s+\w+\s+of\s+(\w+)\s+is\b", re.I | re.M)
BODY = re.compile(r"^\s*package\s+body\s+(\w+)\s+is\b", re.I | re.M)
USE = re.compile(r"\b(?:use|context|entity|new)\s+(\w+)\.(\w+)", re.I)


def scan(path):
    t = strip_comments(Path(path).read_text(encoding="latin-1"))
    defs = {n.lower() for _, rx in UNIT_RES for n in rx.findall(t)}
    needs = {("work", n.lower()) for n in ARCH.findall(t) + BODY.findall(t)}
    refs = {(l.lower(), u.lower()) for l, u in USE.findall(t)}
    return defs, needs | refs


def order_files(files):
    """files: list of (lib, sid, relpath, abspath). Topological order, ties by input order."""
    info = [scan(f[3]) for f in files]
    owner = {}
    for i, (d, _) in enumerate(info):
        for u in d:
            owner.setdefault((files[i][0], u), i)
    deps = []
    for i, (d, n) in enumerate(info):
        s = set()
        for l, u in n:
            j = owner.get((files[i][0] if l == "work" else l, u))
            if j is not None and j != i:
                s.add(j)
        deps.append(s)
    out, done = [], set()
    while len(out) < len(files):
        for i in range(len(files)):
            if i not in done and deps[i] <= done:
                break
        else:  # cycle: should not happen; break it by input order
            i = next(i for i in range(len(files)) if i not in done)
            print("warning: dependency cycle at", files[i][2], file=sys.stderr)
        done.add(i)
        out.append(files[i])
    return out


def gen_vunit(std):
    vr, orr = find_root("vunit"), find_root("osvvm")
    vhdl = vr / "vunit/vhdl"
    files, excl = [], []

    def add(lib, sid, base, absf):
        files.append((lib, sid, str(absf.relative_to(base)), absf))

    def add_files(pattern, filt=True):
        for f in sorted(glob.glob(str(vhdl / pattern))):
            f = Path(f)
            if filt and not vunit_std_ok(f.name, std):
                excl.append(("vunit", str(f.relative_to(vr)), f"file-name standard tag excludes {std} (vunit/builtins.py)"))
            else:
                add("vunit_lib", "vunit", vr, f)

    # Builtins.add_vhdl_builtins
    add_files("data_types/src/*.vhd")
    for k in ("string", "integer_vector"):
        add_files(f"data_types/src/api/external_{k}_pkg.vhd")
    loc = "location_pkg-body-2019p.vhd" if std == "2019" else "location_pkg-body-2008m.vhd"
    add("vunit_lib", "vunit", vr, vhdl / "logging/src" / loc)
    for f in sorted(glob.glob(str(vhdl / "logging/src/*.vhd"))):
        f = Path(f)
        if f.name.startswith("location_pkg-body"):
            if f.name != loc:
                excl.append(("vunit", str(f.relative_to(vr)), f"call-path variant not selected for {std}"))
        elif vunit_std_ok(f.name, std):
            add("vunit_lib", "vunit", vr, f)
        else:
            excl.append(("vunit", str(f.relative_to(vr)), f"file-name standard tag excludes {std}"))
    add_files("*.vhd")
    for d in ("core", "string_ops", "check", "dictionary", "run", "path", "com", "verification_components"):
        add_files(f"{d}/src/*.vhd")
    for f in sorted(glob.glob(str(vhdl / "random/src/*.vhd"))):  # add_source_files: no tag filter
        add("vunit_lib", "vunit", vr, Path(f))
    # _add_osvvm, simulator without the vendor coverage API, with package generics
    skip = {"AlertLogPkg_body_BVUL.vhd": "VUnit skips it", "VendorCovApiPkg_Aldec.vhd": "vendor specific (sim4 only)",
            "ScoreboardPkg_int_c.vhd": "non-generic fallback, unused with generic packages",
            "ScoreboardPkg_slv_c.vhd": "non-generic fallback, unused with generic packages",
            "MemoryPkg_c.vhd": "non-generic fallback, unused with generic packages",
            "MemoryPkg_orig_c.vhd": "non-generic fallback, unused with generic packages"}
    for f in sorted(glob.glob(str(orr / "*.vhd"))):
        f = Path(f)
        if f.name in skip or "2019" in f.name:
            excl.append(("osvvm", f.name, skip.get(f.name, "VUnit skips OSVVM files with 2019 in the name")))
        else:
            add("osvvm", "osvvm", orr, f)
    excl.append(("vunit", "vunit/vhdl/*/test/", "VUnit's own test benches (M4 external suite, not a library)"))
    excl.append(("vunit", "vunit/vhdl/osvvm/", "empty in the sparse checkout (a submodule); OSVVM comes from source 'osvvm'"))
    ordered = order_files(files)
    return ["vunit", "osvvm"], [(l, s, p, std) for l, s, p, _ in ordered], excl, \
        "order: regex dependency scan (package before body, entity before architecture, use/context/new/entity refs)"


# ---------------------------------------------------------------- OSVVM (.pro, Tcl subset)
class Pro:
    def __init__(self, root, std):
        self.root, self.lib, self.cwd, self.files, self.ignored = root, "osvvm", root, [], {}
        v = {"ToolName": simconf.get("sim1", "osvvm_tool_name", "sim1"), "ToolSupportsGenericPackages": True, "Supports2019Interface": True,
             "Supports2019ImpureFunctions": True, "Supports2019FilePath": True, "Supports2019AssertApi": True,
             "Supports2019Integer64Bits": True, "Supports2019Generics": True,
             # sim1 would pick its vendor variant of CoverageVendorApiPkg; use the simulator independent one
             "FunctionalCoverageIntegratedInSimulator": "default",
             "VhdlVersion": int(std), "UseMemoryPkg01": False, "ClockResetVersion": "2026.09",
             "OsvvmDevDerivePackageHeaders": False, "ToolVendor": simconf.get("sim1", "osvvm_tool_vendor", "sim1")}
        self.vars = v

    @staticmethod
    def words(s):
        """Split a Tcl command line into words; {..} and [..] kept verbatim with their delimiters."""
        out, i, n = [], 0, len(s)
        while i < n:
            c = s[i]
            if c in " \t":
                i += 1
                continue
            j, depth = i, 0
            if c in "{[":
                close = "}" if c == "{" else "]"
                while j < n:
                    if s[j] == c: depth += 1
                    elif s[j] == close:
                        depth -= 1
                        if depth == 0: break
                    j += 1
                j += 1
            elif c == '"':
                j = s.index('"', i + 1) + 1
            else:
                while j < n and s[j] not in " \t":
                    j += 1
            out.append(s[i:j])
            i = j
        return out

    @staticmethod
    def commands(text):
        """Yield command lines (continuation by brace depth / backslash), comments dropped."""
        buf, depth = "", 0
        for raw in text.splitlines():
            line = raw
            if not buf and line.lstrip().startswith("#"):
                continue
            line = re.sub(r"\s*;\s*#.*$", "", line) if "#" in line and not buf and depth == 0 else line
            buf += ("\n" if buf else "") + line
            depth = buf.count("{") - buf.count("}") + buf.count("[") - buf.count("]")
            if depth <= 0 and not buf.rstrip().endswith("\\"):
                if buf.strip():
                    yield buf.strip()
                buf, depth = "", 0
        if buf.strip():
            yield buf.strip()

    def subst(self, w):
        w = w.strip('"')
        if w.startswith("[") and w.endswith("]"):
            cmd = self.words(w[1:-1])
            if cmd[0] == "FindOsvvmScriptSettingsPkg":
                return "OsvvmScriptSettingsPkg_default.vhd"
            if cmd[0] == "FindSettingsPkgBody":
                return f"{self.subst(cmd[1])}_default.vhd"
            return None  # NoNullRangeWarning etc.
        return re.sub(r"\$\{?::osvvm::(\w+)\}?", lambda m: str(self.vars[m.group(1)]), w)

    def cond(self, expr):
        def var(m):
            v = self.vars.get(m.group(1))
            if v is None:
                print("osvvm: variable", m.group(1), "unset, taken as false", file=sys.stderr)
                v = False
            return repr(v)
        e = re.sub(r"\[DirectoryExists\s+(\S+?)\]", lambda m: repr((self.cwd / m.group(1)).is_dir()), expr)
        e = re.sub(r"\[FileExists\s+(\S+?)\]", lambda m: repr((self.cwd / m.group(1)).is_file()), e)
        e = re.sub(r"\[string compare\s+(\S+)\s+(\S+)\]", lambda m: f"(({m.group(1)}>{m.group(2)})-({m.group(1)}<{m.group(2)}))", e)
        e = re.sub(r"\$\{?::osvvm::(\w+)\}?", var, e)
        e = e.replace(" ne ", " != ").replace(" eq ", " == ")
        e = e.replace("&&", " and ").replace("||", " or ")
        e = re.sub(r"\b(true|false)\b", lambda m: m.group(1).capitalize(), e)
        e = re.sub(r'"(\d+(?:\.\d+)?)"', lambda m: repr(m.group(1)), e)
        return bool(eval(e, {"__builtins__": {}}, {}))

    def run_text(self, text):
        for cmd in self.commands(text):
            w = self.words(cmd)
            name = w[0]
            if name == "library":
                self.lib = self.subst(w[1])
            elif name == "analyze":
                f = self.subst(w[1])
                self.files.append((self.lib, (self.cwd / f).resolve()))
            elif name == "include":
                self.include(self.subst(w[1]))
            elif name == "ChangeWorkingDirectory":
                self.cwd = (self.cwd / self.subst(w[1])).resolve()
            elif name == "if":
                i = 1
                while i < len(w):
                    if w[i] in ("elseif",):
                        i += 1
                    if w[i] == "else":
                        self.run_text(w[i + 1][1:-1]); break
                    if self.cond(w[i][1:-1]):
                        self.run_text(w[i + 1][1:-1]); break
                    i += 2
                    if i < len(w) and w[i] not in ("elseif", "else"):
                        break
            else:
                self.ignored[name] = self.ignored.get(name, 0) + 1

    def include(self, p):
        t = (self.cwd / p).resolve()
        if t.is_dir():
            for n in (t.name + ".pro", "build.pro"):
                if (t / n).is_file():
                    t = t / n
                    break
        saved = self.cwd, self.lib
        self.cwd = t.parent
        self.run_text(t.read_text(encoding="utf-8", errors="replace"))
        self.cwd, self.lib = saved  # include restores directory; library persists in OSVVM, keep the caller's anyway


def gen_osvvm(std):
    r = find_root("osvvm-libraries")
    p = Pro(r, std)
    p.run_text((r / "OsvvmLibraries.pro").read_text())
    print("osvvm: ignored Tcl commands:", p.ignored, file=sys.stderr)
    files = [(l, "osvvm-libraries", str(f.relative_to(r)), std) for l, f in p.files]
    excl = [("osvvm-libraries", "*/testbench*, */TestCases, */RunAllTests.pro", "test benches and regression scripts, not libraries"),
            ("osvvm-libraries", "osvvm/CoverageVendorApiPkg_NVC.vhd", "simulator specific; the 'default' variant is used"),
            ("osvvm-libraries", "osvvm/deprecated/*", "variants .pro selects only for tools without 2019/generic support")]
    return ["osvvm-libraries"], files, excl, f"order: OsvvmLibraries.pro interpreted with sim1 settings, VHDL {std}"


# ---------------------------------------------------------------- UVVM
def gen_uvvm(std):
    r = find_root("uvvm")
    files = []
    for comp in (r / "script/component_list.txt").read_text().split():
        d = r / comp / "script"
        lib = None
        for line in (d / "compile_order.txt").read_text().splitlines():
            line = line.strip()
            m = re.match(r"#\s*library\s+(\w+)", line)
            if m:
                lib = m.group(1)
            elif line and not line.startswith("#"):
                files.append((lib, "uvvm", str((d / line).resolve().relative_to(r)), std))
    excl = [("uvvm", "*/tb/, */demo/, */sim/", "test benches and demos, not part of any compile_order.txt"),
            ("uvvm", "uvvm_vvc_framework/src_target_dependent/*", "compiled once per VVC library (listed per library), as UVVM's scripts do")]
    return ["uvvm"], files, excl, f"order: script/component_list.txt, each component's script/compile_order.txt, VHDL {std}"


# ---------------------------------------------------------------- hdl-modules
def gen_hdl_modules(std):
    """tsfpga conventions: modules/<name>/{src,rtl} is library <name> (library_name_has_lib_suffix=False).
    Order: regex dependency scan, as for VUnit."""
    r = find_root("hdl-modules")
    files, excl = [], []
    for m in sorted((r / "modules").iterdir()):
        for sub in ("src", "rtl"):
            for f in sorted((m / sub).glob("*.vhd")) if (m / sub).is_dir() else []:
                files.append((m.name, "hdl-modules", str(f.relative_to(r)), f))
    for pat, why in (("modules/*/test/", "test benches (need VUnit)"),
                     ("modules/*/sim/", "simulation-only models, need vunit_lib"),
                     ("modules/*/regs_sim/", "simulation-only, generated by hdl-registers"),
                     ("modules/*/regs_src/", "generated by hdl-registers, not in the repository")):
        excl.append(("hdl-modules", pat, why))
    gen = "modules/dma_axi_write_simple/src/dma_axi_write_simple"
    for suffix in ("", "_axi_lite"):
        excl.append(("hdl-modules", f"{gen}{suffix}.vhd", "needs register packages generated by hdl-registers (not in the repository)"))
    files = [f for f in files if not f[2].startswith(gen) or f[2].endswith("_pkg.vhd")]
    ordered = order_files(files)
    # debounce has `library resync;` and resync needs common: the library must exist when debounce is analyzed.
    late = [f for f in ordered if f[2].endswith("common/src/debounce.vhd")]
    ordered = [f for f in ordered if f not in late] + late
    return ["hdl-modules"], [(l, s, p, std) for l, s, p, _ in ordered], excl, \
        f"order: regex dependency scan; library per module (tsfpga), src/ and rtl/ only, VHDL {std}"


# ---------------------------------------------------------------- yavs lib
def gen_yavs_lib(std):
    t = (REPO / "lib/ORDER.md").read_text()
    blk = re.search(rf"```order-{std}\n(.*?)```", t, re.S).group(1)
    files = [(l, "yavs", "lib/" + f, std) for l, f in (x.split() for x in blk.splitlines() if x.strip())]
    return ["yavs"], files, [], f"order: lib/ORDER.md order-{std} block"


def toml_str(s):
    return '"' + s.replace("\\", "\\\\").replace('"', '\\"') + '"'


def write(name, std, srcs, files, excl, note, flags, extra=None):
    L = [f"# Generated by tools/corpus/gen_manifests.py; do not edit by hand.", f"# {note}",
         f"name = {toml_str(name)}", f'std = "{std}"', "",
         "# Files not compiled, with the reason.", "exclude = ["]
    for s, p, why in excl:
        L.append(f"  {{ source = {toml_str(s)}, path = {toml_str(p)}, reason = {toml_str(why)} }},")
    L += ["]", "", "# Compile order: (library, source, path relative to the source root, std).", "file = ["]
    for l, s, p, st in files:
        L.append(f"  {{ lib = {toml_str(l)}, source = {toml_str(s)}, path = {toml_str(p)}, std = {toml_str(st)} }},")
    L += ["]", "", "# Extra analyze flags per tool (UVVM has non-protected shared variables).", "[flags]"]
    for tool, fl in flags.items():
        L.append(f"{tool} = [{', '.join(toml_str(x) for x in fl)}]")
    for sid in srcs:
        url, commit, why, *lic = SOURCES.get(sid, ("(this repository)", "(see git)", "yavs's own lib/"))
        L += ["", "[[source]]", f"id = {toml_str(sid)}", f"url = {toml_str(url)}", f"commit = {toml_str(commit)}"]
        if lic:
            L.append(f"license = {toml_str(lic[0])}")
        L += [f"# {why}"] + ([] if sid != "yavs" else ["local = true"])
    tb = (extra or {}).get("testbenches")
    if tb:
        L += ["", "# Test benches in the compile list above: how they are run, what passing means, and their top units."]
        L += ["[testbenches]"] + [f"{k} = {toml_str(v)}" for k, v in tb.items() if k != "bench"]
        for b in tb["bench"]:
            L += ["", "[[testbenches.bench]]"] + [f"{k} = {toml_str(v) if isinstance(v, str) else str(v).lower()}" for k, v in b.items()]
    (HERE / f"{name}.toml").write_text("\n".join(L) + "\n")
    print(f"{name}: {len(files)} files, {len(excl)} exclusions")


GEN = {"hdl-modules": gen_hdl_modules, "vunit": gen_vunit, "osvvm": gen_osvvm, "uvvm": gen_uvvm, "yavs-lib": gen_yavs_lib}
FLAGS = {"yavs-lib": {"sim1": [], "yavs": ["--bootstrap"]}, "uvvm": {"sim1": ["--relaxed"], "yavs": ["--relaxed"]}}

SPECS = gen_external.SPECS
VALIDATING = False
validate, save_rejects = gen_external.validate, gen_external.save_rejects
for _n in SPECS:
    GEN[_n] = (lambda n: lambda std: gen_external.gen_ext(n, std, sys.modules[__name__]))(_n)

if __name__ == "__main__":
    ap = argparse.ArgumentParser()
    ap.add_argument("--root", action="append")
    ap.add_argument("--validate", action="store_true", help="run the reference tools over the candidates and record rejects")
    ap.add_argument("names", nargs="*")
    a = ap.parse_args()
    if a.root:
        ROOTS[:0] = a.root
    names = a.names or [f"{c}-{s}" for c in GEN for s in (gen_external.SPECS[c]["stds"] if c in SPECS else ("2008", "2019"))]
    for n in names:
        c, s = n.rsplit("-", 1)
        VALIDATING = a.validate
        r = GEN[c](s)
        srcs, files, excl, note = r[:4]
        fl = FLAGS.get(c, {"sim1": [], "yavs": []})
        if c in SPECS:
            fl = dict({"sim1": [], "yavs": []}, **SPECS[c].get(f"flags-{s}", SPECS[c].get("flags", {})))
        if a.validate and c in SPECS:
            acc, rej, log = validate(n, s, [f for f in files], r[4]["base"], fl)
            save_rejects(n, rej, log)
            VALIDATING = False
            r = GEN[c](s)
            srcs, files, excl, note = r[:4]
        write(n, s, srcs, files, excl, note, fl, r[4] if len(r) > 4 else None)
        if c in SPECS:
            gen_external.notes(n, s, len(files), r[4]["prefix_n"])

#!/usr/bin/env python3
"""Conformance report: runs the evidence suites that need only yavs and the fetched corpora (what CI
regenerates), optionally the ones that need installed reference simulators (--local), and writes
docs/conformance.md and docs/conformance.json from the results plus the committed records.

  tools/conformance/run.py [--yavs BIN] [--out DIR] [--only a,b] [--local] [--llvm] [--check]
  tools/conformance/run.py --report-only --out DIR      # render from the results saved in DIR

Steps (--only names): difftest, discrepancies, external, corpora, uvvm, osvvm, designs; with --local also
refdiff, vhpi (reference simulators configured in tools/sims.local.conf); with --llvm (a build with
the `llvm` feature) also difftest of the auto and llvm tiers and the designs under auto. A --local run stores
what it ran in docs/conformance-local.json with date and commit; CI never writes it. --check renders into --out
and exits 1 when the committed docs/conformance.md differs outside the `Generated:` line.
--waveq re-renders the committed report with the current tools/waveq results (a local record written by tools/waveq).
Needs the corpora: tools/fetch-corpus.sh (YAVS_CORPUS, default ~/.cache/rsim-corpus). Python stdlib only.
"""
import argparse, concurrent.futures as cf, datetime, json, os, re, subprocess, sys, tempfile
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
TOOLS = ROOT / "tools"
DOCS = ROOT / "docs"
MD, JS, LOCAL = DOCS / "conformance.md", DOCS / "conformance.json", DOCS / "conformance-local.json"
GH = "https://github.com/ru551n/yavs"
PY = ["python3", "-I"]
CI_STEPS = ["difftest", "discrepancies", "external", "corpora", "uvvm", "osvvm", "designs"]
LOCAL_STEPS = ["refdiff", "vhpi", "tiers"]
DESIGNS = ["neorv32_dual_full", "neorv32_single_min", "neorv32_single_cache", "grlib_leon3mp"]


def sh(cmd, env=None, check=False):
    """Run cmd; return (exit status, output). A failing check is a result, not an error, unless check."""
    p = subprocess.run([str(c) for c in cmd], capture_output=True, text=True, errors="replace",
                       env={**os.environ, **(env or {})}, cwd=ROOT)
    if check and p.returncode:
        sys.exit(f"{cmd[:3]} failed ({p.returncode}):\n{(p.stdout + p.stderr)[-2000:]}")
    return p.returncode, p.stdout + p.stderr


def git(*a):
    return subprocess.run(["git", "-C", ROOT, *a], capture_output=True, text=True, check=True).stdout.strip()


def jsonl(path):
    return [json.loads(l) for l in Path(path).read_text().splitlines() if l.strip()]


def groups(rows, key=lambda r: r["name"].split("/")[0], ok=lambda r: r["ok"]):
    g = {}
    for r in rows:
        c = g.setdefault(key(r), [0, 0])
        c[0] += bool(ok(r))
        c[1] += 1
    return dict(sorted(g.items()))


# ------------------------------------------------------------------ steps (each returns a JSON-able dict)
class Run:
    def __init__(self, a):
        self.a, self.out = a, Path(a.out)
        self.nosb = ["--no-sandbox"] if a.no_sandbox else []
        # the corpus runner and the VHPI harness start `yavs` from PATH
        self.env = {"YAVS_BIN": str(a.yavs), "YAVS_STD_CACHE": str(self.out / "std"),
                    "PATH": str(Path(a.yavs).parent) + os.pathsep + os.environ["PATH"]}
        self.jobs = str(a.jobs)
        self.corpus = Path(os.environ.get("YAVS_CORPUS") or "~/.cache/rsim-corpus").expanduser()

    def tool(self, *cmd):
        return sh(PY + [TOOLS / cmd[0], *cmd[1:]], self.env)

    def difftest(self, cand):
        res = {}
        for label, extra in [("1 thread", []), ("4 threads", ["--threads", "4"]), ("4 threads shuffled", ["--threads", "4", "--shuffle", "1"])]:
            f = self.out / f"difftest-{cand}-{len(res)}.json"
            self.tool("difftest/difftest.py", "--candidate", cand, "--jobs", self.jobs, "--json", f, *self.nosb, *extra)
            rows = jsonl(f)
            res[label] = {"groups": groups(rows), "failed": [r["name"] for r in rows if not r["ok"]]}
        return res

    def step_difftest(self):
        return {"interp": self.difftest("yavs-interp")}

    def step_discrepancies(self):
        rc, out = self.tool("discrepancies/run.py", "--sims", "yavs", "--check", *self.nosb)
        cases = [d for d in (ROOT / "tests/discrepancies").iterdir() if (d / "case.conf").exists()]
        return {"cases": len(cases), "yavs_matches_expected": rc == 0, "output_tail": "" if rc == 0 else out[-600:]}

    def step_external(self):
        res = {}
        for s in ("sim1", "sim2"):
            f = self.out / f"external-{s}.json"
            self.tool("difftest/external.py", s, "--oracle", "--jobs", self.jobs, "--json", f, *self.nosb)
            rows = jsonl(f)
            classes = {}
            for r in rows:
                classes[r["class"]] = classes.get(r["class"], 0) + 1
            want = set((TOOLS / f"difftest/external-{s}.pass").read_text().split())
            got = {r["name"] for r in rows if r["class"] in ("pass", "early")}
            res[s] = {"tests": len(rows), "classes": dict(sorted(classes.items(), key=lambda kv: -kv[1])),
                      "pass_list": len(want), "pass_list_regressions": sorted(want - got)}
        return res

    def step_corpora(self):
        def one(t):
            m = t.stem
            rc, out = sh(PY + [TOOLS / "corpus/run.py", m, "--no-wrap", "--keep-going", "--out", self.out / "corpus" / m], self.env)
            mm = re.search(r"^\S+ \[yavs\]: (\d+) PASS, (\d+) FAIL of (\d+)", out, re.M)
            if not mm:
                return m, ({"skipped": "a vendor library directory is not available"} if "skipped" in out else {"error": out[-300:]})
            return m, {"pass": int(mm[1]), "fail": int(mm[2]), "total": int(mm[3]), "matches_expected": rc == 0}
        with cf.ThreadPoolExecutor(2) as ex:
            return dict(ex.map(one, sorted((TOOLS / "corpus").glob("*.toml"))))

    def suite(self, name, cmd, status_col):
        d = self.out / name
        rc, text = sh(cmd + ["--out", d], self.env)
        if not (d / "results.tsv").exists():
            sys.exit(f"{cmd[2]} wrote no results.tsv (exit {rc}):\n{text[-1500:]}")
        rows = [l.split("\t") for l in (d / "results.tsv").read_text().splitlines()]
        res = {"runs": len(rows)}
        for r in rows:
            res[r[status_col].lower()] = res.get(r[status_col].lower(), 0) + 1
        return res

    def step_uvvm(self):
        return self.suite("uvvm", PY + [TOOLS / "suites/uvvm.py", "--yavs", self.a.yavs, "-j", "4", "--no-wrap"], 3)

    def step_osvvm(self):
        return {std: self.suite(f"osvvm-{std}", PY + [TOOLS / "suites/pro.py", "--std", std, "--root", self.corpus / "osvvm-libraries", "--yavs", self.a.yavs, "--jobs", "4", "--no-wrap"], 3)
                for std in ("2008", "2019")}

    def step_designs(self, tiers="interp", name="designs"):
        d = self.out / name
        rc, out = sh(PY + [TOOLS / "realbench/run.py", self.a.yavs, "--tests", ",".join(DESIGNS), "--tiers", tiers, "--phases", "run",
                           "--runs", "0", "--no-instructions", "--cores", self.a.core, "--out", d], self.env)
        if rc:
            sys.exit(f"realbench failed:\n{out[-1500:]}")
        j = json.loads(sorted((d / "results").glob("*.json"))[-1].read_text())
        return {n: {t: bool(c["ok"]) for t, c in v["cells"].items()} for n, v in j["tests"].items()}

    # ---- local-only (reference simulators configured, or a build with the llvm feature)
    def step_tiers(self):
        return {"auto": self.difftest("yavs-auto"), "llvm": self.difftest("yavs-llvm"), "designs": self.step_designs("auto", "designs-auto")}

    def step_refdiff(self):
        res = {}
        for s in ("sim1", "sim2"):
            f = self.out / f"refdiff-{s}.json"
            self.tool("difftest/difftest.py", "--candidate", s, "--jobs", "4", "--json", f)
            rows = jsonl(f)
            res[s] = {"agree": sum(r["ok"] for r in rows), "tests": len(rows)}
        return res

    def step_vhpi(self):
        rc, out = sh(PY + [TOOLS / "vhpi-conformance/run.py", "--out", self.out / "vhpi"], self.env)
        lines = [l for l in out.splitlines() if re.match(r"^(PASS|DIFF)\b", l)]
        d = self.out / "vhpi"
        total = sum(len(f.read_text().splitlines()) for f in d.glob("*.sim1.trace"))
        diffs = [l for f in d.glob("*.diff") for l in f.read_text().splitlines() if l[:1] and l[0] in "+-" and l[:3] not in ("+++", "---")]
        return {"plugins": len(lines), "traces_equal": sum(l.startswith("PASS") for l in lines), "sim1_lines": total,
                "sim1_lines_not_reproduced": sum(l[0] == "-" for l in diffs), "yavs_extra_lines": sum(l[0] == "+" for l in diffs)}


def collect(a):
    out = Path(a.out)
    out.mkdir(parents=True, exist_ok=True)
    r = Run(a)
    names = a.only.split(",") if a.only else CI_STEPS + ([s for s in LOCAL_STEPS if s != "tiers" or a.llvm] if a.local else [])
    res = {}
    for n in names:
        print(f"conformance: {n}", flush=True)
        res[n] = getattr(r, "step_" + n)()
        (out / f"{n}.json").write_text(json.dumps(res[n], indent=1))
    return res


# ------------------------------------------------------------------ static facts
def meta(a):
    ver = sh([a.yavs, "--version"])[1].split()[-1] if a.yavs else "?"
    # a local run on commits not yet pushed names the last commit of origin/main they sit on (the sources that were run)
    commit = os.environ.get("GITHUB_SHA") or git("merge-base", "HEAD", "origin/main")
    dirty = any(not l[3:].startswith("docs/conformance") for l in git("status", "--porcelain").splitlines())  # the outputs of this tool do not count
    return {"version": ver, "commit": commit[:10] + ("-dirty" if dirty else ""), "commit_date": git("show", "-s", "--format=%cs", commit)}


def discrepancy_table():
    rows, seen = [], set()
    for line in (ROOT / "docs/design/simulator-discrepancies.md").read_text().splitlines():
        if line.startswith("| `"):
            c = [x.strip() for x in re.split(r"(?<!\\)\|", line)[1:-1]]
            if len(c) >= 10 and (ROOT / "tests/discrepancies" / c[0].strip("`") / "case.conf").exists() and c[0] not in seen:
                seen.add(c[0])  # case, construct, lrm, kind, sim1..sim4, yavs, choice, decided
                rows.append({"case": c[0].strip("`"), "lrm": c[2], "kind": c[3], "yavs": c[8], "decided": c[10] if len(c) > 10 else ""})
    return rows


def kind_of(k):
    for p in ("LRM clear", "LRM ambiguity", "reference bug", "library availability"):
        if k.startswith(p):
            return p
    return "other"


def corpus_refs():
    """Recorded results of the first reference simulator on the same files (its .sim1.expected lists)."""
    res = {}
    for f in sorted((TOOLS / "corpus").glob("*.sim1.expected")):
        ls = [l for l in f.read_text().splitlines() if l.startswith(("PASS ", "FAIL "))]
        res[f.name.removesuffix(".sim1.expected")] = {"pass": sum(l.startswith("PASS") for l in ls), "total": len(ls),
                                                      "recorded": git("log", "-1", "--format=%cs", "--", str(f))}
    return res


def ci_platforms():
    jobs = re.findall(r"^  (\w+):\n(?:    name: .*\n)?(?:    strategy:\n      fail-fast: \w+\n      matrix:\n        os: \[(.*?)\]\n)?", (ROOT / ".github/workflows/ci.yml").read_text(), re.M)
    return {j: [o.strip() for o in os_.split(",")] for j, os_ in jobs if os_}


LIMITS = [
    ("Verilog and mixed-language designs", f"not supported yet; the plan is [#776]({GH}/issues/776)"),
    ("Encrypted IP", f"an encrypted VHDL file stops with E0012 ([#775]({GH}/issues/775)); the contents are never guessed"),
    ("PSL", f"a subset; the rest is skipped with warning W0004 or listed in the tracking issue [#445]({GH}/issues/445)"),
    ("SDF and VITAL timing, SystemC, macOS", "out of scope (docs/plan.md, \"Out of scope\")"),
    ("Platforms", "Linux x86_64, Linux aarch64 and Windows x86_64 only"),
    ("LLVM tier", "built only with the `llvm` feature (LLVM 21); the default build runs the interpreter tier. CI runs a slice of the corpus under the LLVM tier on the three platforms"),
    ("Reference simulators", "the comparison is against two established open-source VHDL simulators (sim3 and sim4, two commercial simulators, vote in part of the discrepancy catalogue); where the language reference is unclear yavs is permissive and the choice is recorded below"),
    ("What a pass means", "a pass is agreement with a recorded reference output or with a suite's own verdict. It is evidence, not a proof of correctness; each known disagreement is listed in the catalogue"),
]


# ------------------------------------------------------------------ rendering
def table(head, rows, right=()):
    sep = ["---:" if i in right else "---" for i in range(len(head))]
    return "\n".join("| " + " | ".join(map(str, r)) + " |" for r in [head, sep] + rows) + "\n"


def frac(p, t):
    return f"{p}/{t}" if p == t else f"**{p}/{t}**"


def render(R):
    ci, loc, st, m = R["ci"], R["local"]["records"], R["static"], R["generated"]
    o = [f"""# yavs conformance report

Generated: yavs {m['version']} at commit `{m['commit']}` ({m['commit_date']}) by `tools/conformance/run.py`; do not edit by hand.

This report collects the evidence that yavs behaves as the VHDL language reference and the established open-source simulators do.
Nothing here is a proof of correctness. Every number comes from a command in the "Reproduce" section; sections marked **CI** are
regenerated by the `conformance` workflow on every push to `main` and on release tags (artifact `conformance-report`), sections
marked **local** need installed reference simulators or vendor libraries and were run by hand on the date and commit shown.
The references are called sim1 and sim2 (two established open-source VHDL simulators).
"""]
    # --- headline
    dt = ci.get("difftest", {}).get("interp", {})
    t4 = dt.get("4 threads shuffled", {}).get("groups", {})
    allg = lambda d: [sum(v[0] for v in d.values()), sum(v[1] for v in d.values())]
    h = []
    if t4:
        h.append(("Differential corpus, interpreter, 4 threads shuffled", "{}/{} testbenches equal to the recorded sim1 output, identical at 1 and 4 threads".format(*allg(t4))))
    for s, v in ci.get("external", {}).items():
        run = v["tests"] - v["classes"].get("skip", 0)
        h.append((f"{s}'s own regression tests", f"{v['classes'].get('pass', 0) + v['classes'].get('early', 0)} of {run} runnable tests pass the suite's own verdict ({v['pass_list']} on a gated pass list, {len(v['pass_list_regressions'])} regressions); {v['classes'].get('skip', 0)} skipped (other languages, VHPI, scripts)"))
    if "corpora" in ci:
        cs = [v for v in ci["corpora"].values() if "total" in v]
        gated = "all" if all(v["matches_expected"] for v in cs) else "NOT ALL"
        h.append(("Analysis corpora (third-party VHDL code)", f"{sum(v['pass'] for v in cs)} of {sum(v['total'] for v in cs)} files analyse, {len(cs)} corpora, {gated} equal to the gated lists"))
    if "discrepancies" in ci:
        h.append(("Known discrepancy cases", f"{ci['discrepancies']['cases']} reproducers, yavs output equals the recorded one: {'yes' if ci['discrepancies']['yavs_matches_expected'] else 'NO'}"))
    if "uvvm" in ci:
        u = ci["uvvm"]
        h.append(("UVVM's own testbenches", f"{u.get('pass', 0)} of {u['runs']} runs pass ({u.get('skip', 0)} skipped: vendor libraries absent)"))
    if "osvvm" in ci:
        h.append(("OSVVM's own regression", ", ".join(f"{s}: {v.get('pass', 0)}/{v['runs']}" for s, v in ci["osvvm"].items())))
    if "designs" in ci:
        h.append(("Real designs run to completion (neorv32 x3, GRLIB LEON3)", "{}/{}".format(sum(all(t.values()) for t in ci["designs"].values()), len(ci["designs"]))))
    w = R.get("waveq")
    if w:
        h.append(("Waveform equivalence with sim1 (local, real designs)", f"{w['designs']} designs compared signal by signal and time step by time step, interpreter: {w['equal']} of {w['signals']} signals equal over {w['changes']} value changes; "
                  f"{w['different']} signals differ, {w['different'] - w['unexplained']} of them explained by reference-simulator behaviour the language reference leaves open, {w['unexplained']} unexplained"))
    for k, v in loc.items():
        if k == "vunit-own":
            h.append(("VUnit's own VHDL suites (local)", f"{v['data']['pass']} of {v['data']['pass'] + v['data']['fail']} tests pass, 11 suites"))
    o.append("## Headline\n\n" + table(["Evidence", "Result"], [list(x) for x in h]))
    o.append("""## Reproduce

```sh
tools/fetch-corpus.sh                        # external corpora at pinned commits, ~/.cache/rsim-corpus
cargo build --release -p yavs                # add --features llvm for the LLVM tier (needs LLVM 21)
tools/conformance/run.py --yavs target/release/yavs --out ~/.cache/yavs-conformance          # the CI steps
tools/conformance/run.py --yavs target/release/yavs --out ~/.cache/yavs-conformance --local --llvm   # plus the local ones
tools/conformance/run.py --check --yavs target/release/yavs --out ~/.cache/yavs-conformance # is the committed report current?
```

The local steps need the reference simulators configured in the untracked `tools/sims.local.conf` (`tools/AGENTS.md` section 2a).
A `--local` run rewrites `docs/conformance-local.json`; commit it together with the regenerated report.
""")
    # --- provenance table
    prov = [["Differential corpus, interpreter", "CI", "`tools/difftest/difftest.py --candidate yavs-interp [--threads 4 [--shuffle 1]]`"],
            ["Known discrepancy cases", "CI", "`tools/discrepancies/run.py --sims yavs --check`"],
            ["Reference simulators' own tests", "CI", "`tools/difftest/external.py sim1|sim2 --oracle`"],
            ["Analysis corpora", "CI", "`tools/corpus/run.py <manifest> --keep-going`"],
            ["UVVM, OSVVM suites", "CI", "`tools/suites/uvvm.py`, `tools/suites/pro.py --std 2008|2019`"],
            ["Real designs", "CI", "`tools/realbench/run.py BIN --tests ... --runs 0`"]]
    if w:
        prov.append(["Waveform equivalence", f"local, {w['run']['date']} at `{w['run']['commit']}`", f"`{w['run']['command']}`"])
    for k, v in loc.items():
        prov.append([v["title"], f"local, {v['date']} at `{v['commit']}`", f"`{v['command']}`"])
    o.append("## What was run where\n\n" + table(["Evidence", "Where", "Command"], prov))
    # --- difftest
    if dt:
        o.append("## Differential simulation (CI)\n\nEvery testbench of `tests/corpus` runs on yavs and its normalised output is compared with the "
                 "recorded output of sim1 (`tools/difftest/README.md`). Strict mode: no ignored fields unless the test says so with a comment. "
                 "The 4-thread columns also require output byte-identical to the 1-thread run.\n")
        names = sorted(dt["1 thread"]["groups"])
        rows = []
        for g in names + ["total"]:
            row = [g]
            for lab in ("1 thread", "4 threads", "4 threads shuffled"):
                gg = dt[lab]["groups"]
                row.append(frac(*(allg(gg) if g == "total" else gg.get(g, [0, 0]))))
            rows.append(row)
        o.append(table(["group", "1 thread", "4 threads", "4 threads shuffled"], rows))
        fails = sorted({f for lab in dt.values() for f in lab["failed"]})
        o.append("Failing: " + (", ".join(f"`{f}`" for f in fails) if fails else "none") + ".\n")
    for k, v in loc.items():
        if k == "tiers":
            o.append(f"The compiled tiers must give the same output as the interpreter (local, {v['date']} at `{v['commit']}`, a build with the `llvm` feature): " +
                     "; ".join(f"tier `{t}`: " + ", ".join(f"{lab} {frac(*allg(x['groups']))}" for lab, x in v["data"][t].items()) for t in ("auto", "llvm")) + ".\n")
        if k == "refdiff":
            o.append(f"The references against the recorded sim1 outputs (local, {v['date']} at `{v['commit']}`): " +
                     ", ".join(f"{s} {d['agree']}/{d['tests']}" for s, d in v["data"].items()) + ". "
                     "This shows how far the oracle is itself stable across simulators.\n")
    # --- external
    if "external" in ci:
        o.append("## The reference simulators' own regression tests (CI)\n\nEach reference simulator ships a suite of self-checking tests. They run in place from the "
                 "fetched checkout on yavs and are judged by the suite's own verdict. The pass lists (`tools/difftest/external-*.pass`) are gated: "
                 "a listed test that stops passing fails CI. The others fail the suite's own verdict, fail to analyse or elaborate, or are skipped (other languages, VHPI, scripts); "
                 "the suites contain tests of features yavs does not implement and tests of reference-specific behaviour.\n")
        o.append(table(["suite", "tests", "pass", "gated pass list", "regressions"] , [[s, v["tests"], v["classes"].get("pass", 0) + v["classes"].get("early", 0), v["pass_list"], len(v["pass_list_regressions"])] for s, v in ci["external"].items()], right=(1, 2, 3, 4)))
        o.append("Not passing: " + "; ".join(f"{s}: {v['tests'] - v['classes'].get('pass', 0) - v['classes'].get('early', 0) - v['classes'].get('skip', 0)}" for s, v in ci["external"].items()) +
                 " (the classes of each are in `docs/conformance.json`; a timeout under load moves a test between them, so the report does not).\n")
    # --- discrepancies
    rows = st["discrepancies"]
    cnt = {}
    for r in rows:
        cnt[kind_of(r["kind"])] = cnt.get(kind_of(r["kind"]), 0) + 1
    o.append(f"""## Known discrepancies (CI for yavs's side)

`docs/design/simulator-discrepancies.md` lists every case where the references disagree with each other, with the language reference or with yavs:
{len(rows)} catalogued behaviours, each with a reproducer under `tests/discrepancies/<case>/`. CI runs every reproducer on yavs and fails when yavs's output
differs from the recorded one ({'matches' if ci.get('discrepancies', {}).get('yavs_matches_expected') else 'checked by CI'}). Kinds: """ + ", ".join(f"{k} {n}" for k, n in sorted(cnt.items(), key=lambda kv: -kv[1])) + """.

Policy (user decision, `docs/plan.md`): where the language reference is clear, yavs follows it even against all references; where it is unclear yavs is permissive
and follows the majority of the references that run the construct. The verdicts of each reference are in the catalogue; the table gives the clause, the kind,
yavs's verdict and who decided.

<details><summary>All catalogued cases</summary>

""" + table(["case", "LRM", "kind", "yavs", "decided"], [[f"[`{r['case']}`]({GH}/tree/main/tests/discrepancies/{r['case']})", r["lrm"][:60], kind_of(r["kind"]), r["yavs"][:40], r["decided"][:70]] for r in rows]) + "\n</details>\n")
    # --- corpora
    if "corpora" in ci:
        refs = st["corpus_refs"]
        rows = []
        for k, v in sorted(ci["corpora"].items()):
            rf = refs.get(k)
            rows.append([k] + ([frac(v["pass"], v["total"]), "yes" if v["matches_expected"] else "**no**"] if "total" in v else [v.get("skipped") or "error", ""]) +
                        [f"{rf['pass']}/{rf['total']}" if rf else "-"])
        o.append("## Analysis corpora (CI)\n\nThird-party VHDL libraries and designs, fetched at pinned commits, analysed file by file in build order. "
                 "A file counts as passing when yavs analyses it; the gated list (`tools/corpus/<name>.expected`) must match exactly, in both directions. "
                 "The last column is sim1 on the same files (recorded list `<name>.sim1.expected`). Failures are mostly constructs the corpus uses that yavs rejects "
                 "on purpose or does not support yet, each tracked as an issue.\n\n" + table(["corpus", "yavs passes", "equals gated list", "sim1 passes"], rows, right=(1, 3)))
    # --- suites
    if "uvvm" in ci or "osvvm" in ci or "vunit-own" in loc:
        rows = []
        if "uvvm" in ci:
            u = ci["uvvm"]
            rows.append(["UVVM maintenance testbenches (CI)", u["runs"], u.get("pass", 0), u.get("fail", 0), u.get("skip", 0), "one run per entity, architecture and generic set the project's own script lists"])
        for s, v in ci.get("osvvm", {}).items():
            rows.append([f"OSVVM `.pro` regression, VHDL-{s} (CI)", v["runs"], v.get("pass", 0), v.get("fail", 0) + v.get("nosummary", 0) + v.get("timeout", 0), v.get("skip", 0), "pass = exit 0 and the library's `DONE PASSED` line"])
        for k in ("vunit-own", "cocotb"):
            if k in loc:
                v = loc[k]
                d = v["data"]
                rows.append([f"{v['title']} (local, {v['date']} at `{v['commit']}`)", d["pass"] + d["fail"] + d.get("skip", 0), d["pass"], d["fail"], d.get("skip", 0), v["note"]])
        o.append("## Verification libraries (UVVM, OSVVM, VUnit)\n\nThe libraries' own testbenches and regression suites run on yavs, each with the library's verdict.\n\n" + table(["suite", "runs", "pass", "fail", "skip", "note"], rows, right=(1, 2, 3, 4)))
    # --- vhpi
    if "vhpi" in loc:
        v = loc["vhpi"]
        d = v["data"]
        o.append(f"## VHPI conformance (local, {v['date']} at `{v['commit']}`)\n\nSmall C plugins call the VHPI functions in a fixed order on small designs ({d['plugins']} plugin runs); the traces from yavs and sim1 are diffed "
                 f"(`tools/vhpi-conformance`). {d['traces_equal']} traces are identical. Of sim1's {d['sim1_lines']} trace lines yavs reproduces {d['sim1_lines'] - d['sim1_lines_not_reproduced']}; "
                 f"{d['sim1_lines_not_reproduced']} differ and yavs prints {d['yavs_extra_lines']} lines sim1 does not. Differences are judged one by one: many are properties or callbacks "
                 "that sim1 reports as unsupported and yavs provides, some are deliberate (`tools/vhpi-conformance/AGENTS.md`), real defects become issues.\n")
    else:
        o.append("## VHPI conformance\n\nNot in this report: run `tools/vhpi-conformance/run.py` (needs gcc and sim1); see `tools/vhpi-conformance/AGENTS.md`.\n")
    # --- designs
    if "designs" in ci:
        o.append("## Real designs (CI)\n\nComplete designs, run with the interpreter. neorv32: a RISC-V system running its built-in program until the stop time, three configurations (pass = the run finishes with status 0). "
                 "GRLIB LEON3: the template design boots from its PROM image and runs GRLIB's own system test (pass = status 0 and the line `Test passed`).\n\n" +
                 table(["design", *sorted({t for v in ci["designs"].values() for t in v})], [[n, *["pass" if v.get(t) else "**FAIL**" for t in sorted({t for v in ci["designs"].values() for t in v})]] for n, v in ci["designs"].items()]))
    if "tiers" in loc:
        v = loc["tiers"]
        o.append(f"Under the `auto` tier (local, {v['date']} at `{v['commit']}`): " + ", ".join(f"{n} {'pass' if all(t.values()) else '**FAIL**'}" for n, t in v["data"]["designs"].items()) + ".\n")
    o.append("## Waveform equivalence on real designs\n\n" + R["trust2"] + "\n")
    # --- platforms and limits
    o.append("## Platforms covered by CI\n\n" + table(["job", "runs on"], [[f"`{j}`", ", ".join(os_)] for j, os_ in st["platforms"].items()]) +
             "\nThe `check` job is `cargo fmt`, `clippy -D warnings` and the whole test suite of the workspace; the `llvm` job the same for the LLVM tier plus a simulation smoke run. "
             "The `corpus` and `conformance` jobs run on Linux x86_64 only.\n")
    o.append("## Known limits\n\n" + table(["area", "state"], [list(x) for x in LIMITS]))
    return "\n".join(o)


TRUST2_SLOT = "Not yet in this report. This section is reserved for the waveform comparison of yavs against the reference simulators on neorv32 and GRLIB LEON3 (tracked separately)."


def build(a, ci, gen):
    loc = json.loads(LOCAL.read_text()) if LOCAL.exists() else {"records": {}}
    trust2 = TOOLS / "waveq/RESULTS.md"  # written by tools/waveq (#781)
    R = {"generated": gen, "ci": ci, "local": loc,
         "static": {"discrepancies": discrepancy_table(), "corpus_refs": corpus_refs(), "platforms": ci_platforms()},
         "trust2": trust2.read_text().strip() if trust2.exists() else TRUST2_SLOT, "waveq": waveq_summary()}
    return R


def strip_generated(t):
    return "\n".join(l for l in t.splitlines() if not l.startswith("Generated:"))


def waveq_summary():
    """Totals of tools/waveq/results.json for the interpreter against sim1 (the tool's own file, a local record)."""
    f = TOOLS / "waveq/results.json"
    if not f.exists():
        return None
    j = json.loads(f.read_text())
    rows = [r for r in j["rows"] if r["a"] == "yavs-interp"]
    explained = sum(n for r in rows for n in r["verdicts"].values())
    return {"run": j["run"], "designs": len(rows), "signals": sum(r["signals"] for r in rows), "equal": sum(r["equal"] for r in rows),
            "changes": sum(r["changes"] for r in rows), "different": sum(r["different"] for r in rows), "unexplained": sum(r["different"] for r in rows) - explained}


def refresh_waveq():
    """Re-render the committed report with the current tools/waveq results; every other number stays as committed."""
    R = json.loads(JS.read_text())
    R["trust2"] = (TOOLS / "waveq/RESULTS.md").read_text().strip()
    R["waveq"] = waveq_summary()
    MD.write_text(render(R))
    JS.write_text(json.dumps(R, indent=1, sort_keys=True) + "\n")
    print(f"updated the waveform parts of {MD}")


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--yavs", default=os.environ.get("YAVS_BIN", "yavs"))
    ap.add_argument("--out", default=os.path.expanduser("~/.cache/yavs-conformance"))
    ap.add_argument("--only", help="comma list of steps")
    ap.add_argument("--local", action="store_true", help="also run the steps that need reference simulators; store them in docs/conformance-local.json")
    ap.add_argument("--llvm", action="store_true", help="the binary has the llvm feature: also the auto and llvm tiers")
    ap.add_argument("--check", action="store_true", help="render into --out and compare with the committed report")
    ap.add_argument("--report-only", action="store_true", help="render from the step results already in --out")
    ap.add_argument("--waveq", action="store_true", help="only refresh the waveform section of the committed report from tools/waveq/RESULTS.md")
    ap.add_argument("--no-sandbox", action="store_true", help="no systemd-run around the tools (CI)")
    ap.add_argument("--jobs", type=int, default=4)
    ap.add_argument("--core", default="1", help="cpu for the design runs (not timed)")
    a = ap.parse_args()
    a.yavs = str(Path(a.yavs).resolve()) if "/" in a.yavs else a.yavs
    if a.waveq:
        return refresh_waveq()
    out = Path(a.out)
    if a.report_only:
        ci = {p.stem: json.loads(p.read_text()) for p in out.glob("*.json") if p.stem in CI_STEPS + LOCAL_STEPS}
    else:
        ci = collect(a)
    gen = meta(a)  # before any file of the tree is written
    local_now = {k: ci.pop(k) for k in LOCAL_STEPS if k in ci}
    if local_now and not a.report_only:
        loc = json.loads(LOCAL.read_text()) if LOCAL.exists() else {"records": {}}
        m = gen
        cmds = {"refdiff": "tools/difftest/difftest.py --candidate sim1|sim2", "vhpi": "tools/vhpi-conformance/run.py",
                "tiers": "tools/conformance/run.py --local --llvm --only tiers (yavs built with --features llvm)"}
        titles = {"refdiff": "References against the recorded outputs", "vhpi": "VHPI conformance", "tiers": "The auto and llvm tiers"}
        for k, d in local_now.items():
            loc["records"][k] = {"title": titles[k], "date": datetime.date.today().isoformat(), "commit": m["commit"], "version": m["version"], "command": cmds[k], "data": d, "note": ""}
        LOCAL.write_text(json.dumps(loc, indent=1, sort_keys=True) + "\n")
    R = build(a, ci, gen)
    md, js = render(R), json.dumps(R, indent=1, sort_keys=True) + "\n"
    (out / "conformance.md").write_text(md)  # the CI artifact
    (out / "conformance.json").write_text(js)
    if a.check:
        bad = strip_generated(MD.read_text()) != strip_generated(md) if MD.exists() else True
        if bad:
            import difflib
            sys.stdout.writelines(difflib.unified_diff(strip_generated(MD.read_text()).splitlines(True) if MD.exists() else [], strip_generated(md).splitlines(True), "committed", "regenerated", n=1))
            sys.exit("docs/conformance.md is stale: run tools/conformance/run.py and commit the result")
        print("docs/conformance.md is current")
        return
    MD.write_text(md)
    JS.write_text(js)
    print(f"wrote {MD} and {JS}")


if __name__ == "__main__":
    main()

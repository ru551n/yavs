#!/usr/bin/env python3
"""Process replay: one hot process of a real run, alone, in seconds (AGENTS.md section 8b).

  replay.py BIN TARGET [--check] [--tier auto] [--core N] [--until T] [--keep]
  replay.py --list
  replay.py [BIN] --self-test     (with BIN also a recording and a replay of a small design)

A real run of the target's test records, per delta cycle, every change of the `in` ports of the instance that holds the process (a VHPI
plugin, replay/record.c; the run itself is the unchanged design). A generated testbench instantiates that entity alone, with the same
generics, and drives its ports from the recording, so the processes of the entity run their own code on the inputs and wakes they had.
The replay runs without plugins, so the code, its size and the compile decision are those of a plain run. --check adds the validation:
the process's own symbols in the replay against the same symbols inside the full run, by perf samples. Python is stdlib only; run with
`python3 -I`.
"""
import argparse, hashlib, importlib.util, json, os, re, shlex, shutil, subprocess, sys, time, tomllib
from collections import defaultdict
from pathlib import Path

HERE = Path(__file__).resolve().parent
_spec = importlib.util.spec_from_file_location("realbench_run", HERE / "run.py")
R = importlib.util.module_from_spec(_spec)
_spec.loader.exec_module(R)
_wspec = importlib.util.spec_from_file_location("realbench_window", HERE / "window.py")
W = importlib.util.module_from_spec(_wspec)
_wspec.loader.exec_module(W)
Fail = R.Fail

# target -> test, instance holding the process, process label, end of the recording. The label is the name of the process's function.
TARGETS = {
    "execute1_1": ("microwatt_coremark", "microwatt_bench_tb.soc0.processors(0).core.execute1_0", "execute1_1", "800us"),
    "execute1_dp": ("microwatt_coremark", "microwatt_bench_tb.soc0.processors(0).core.execute1_0", "execute1_dp", "800us"),  # compiled at auto: the symbol check
    "decode2_1": ("microwatt_coremark", "microwatt_bench_tb.soc0.processors(0).core.decode2_0", "decode2_1", "800us"),
    "dctrl": ("grlib_leon3mp", "testbench.d3.l3.cpu(0).u0.leon3x0.vhdl.p0.c0mmu.dcache0", "dctrl", "700us"),
    "iu": ("grlib_leon3mp", "testbench.d3.l3.cpu(0).u0.leon3x0.vhdl.p0.iu", "comb", "700us"),
}
MEM = "8G"  # rule: every run that can grow is capped
PERIOD = {"instructions": 1000003, "cycles": 1000003, "L1-icache-load-misses": 10007, "branch-misses": 10007}  # user space, a period per event
UNTIMED = "0-7,16-23"  # recording, analysis and warm-ups stay off the timing cores


# ---- the entity header and the generated testbench ----

def strip_comments(text):
    """VHDL text with comments blanked (strings kept), so the header parser sees code only."""
    return re.sub(r'"(?:[^"\n]|"")*"|--[^\n]*', lambda m: m.group(0) if m.group(0).startswith('"') else " " * len(m.group(0)), text)


def paren_body(text, at):
    """The text inside the parenthesis that opens at or after `at`, and the index after its close."""
    i = text.index("(", at)
    depth = 0
    for j in range(i, len(text)):
        depth += {"(": 1, ")": -1}.get(text[j], 0)
        if depth == 0:
            return text[i + 1:j], j + 1
    raise Fail("unbalanced parentheses in an entity header")


def split_items(body):
    out, depth, cur = [], 0, ""
    for ch in body:
        depth += {"(": 1, ")": -1}.get(ch, 0)
        if ch == ";" and depth == 0:
            out.append(cur)
            cur = ""
        else:
            cur += ch
    return [x.strip() for x in out + [cur] if x.strip()]


def split_default(s):
    depth = 0
    for i in range(len(s) - 1):
        depth += {"(": 1, ")": -1}.get(s[i], 0)
        if depth == 0 and s[i:i + 2] == ":=":
            return s[:i].strip(), s[i + 2:].strip()
    return s.strip(), None


def entity_header(text, name):
    """(context clause, generics [(name, type, default)], ports [(name, mode, subtype)]) of entity `name` in `text`."""
    code = strip_comments(text)
    m = re.search(rf"\bentity\s+{re.escape(name)}\s+is\b", code, re.I)
    if not m:
        raise Fail(f"entity {name} not found in its source file")
    context = [re.sub(r"\s+", " ", c.strip()) for c in re.findall(r"\b(?:library|use|context)\s[^;]*;", code[:m.start()], re.I)]
    end = re.search(r"\bend\b", code[m.end():], re.I)
    head = code[m.end():m.end() + (end.start() if end else len(code))]
    gens, ports = [], []
    g = re.match(r"\s*generic\b", head, re.I)
    rest = 0
    if g:
        body, rest = paren_body(head, g.end())
        for item in split_items(body):
            names, _, ty = item.partition(":")
            ty, default = split_default(ty)
            for n in names.replace("constant", " ").split(","):
                gens.append((n.strip(), ty, default))
    p = re.search(r"\bport\b", head[rest:], re.I)
    if p:
        body, _ = paren_body(head, rest + p.end())
        for item in split_items(body):
            names, _, rhs = item.partition(":")
            rhs = split_default(rhs)[0]
            mm = re.match(r"\s*(in|out|inout|buffer|linkage)\b", rhs, re.I)
            mode, sub = (mm.group(1).lower(), rhs[mm.end():].strip()) if mm else ("in", rhs.strip())
            for n in names.replace("signal", " ").split(","):
                ports.append((n.strip(), mode, sub))
    return context, gens, ports


def parse_ports_file(text):
    """The recorder's description: instance, (library, entity, architecture), source file, generics, ports, leaves (index, kind, name)."""
    d = {"gens": {}, "ports": [], "leaves": []}
    for line in text.splitlines():
        tag, _, rest = line.partition(" ")
        if tag == "I":
            d["inst"] = rest
        elif tag == "E":
            d["lib"], d["entity"], d["arch"] = rest.split()
        elif tag == "F":
            d["file"] = rest
        elif tag == "G":
            n, _, v = rest.partition(" ")
            k, _, val = v.partition(" ")
            d["gens"][n.lower()] = (k, val)
        elif tag == "P":
            n, mode = rest.split()
            d["ports"].append((n.lower(), mode))
        elif tag == "L":
            i, k, n, ty = rest.split(" ")
            d["leaves"].append((int(i), k, n.lower(), ty.lower()))
    return d


def literal(kind, val, ty):
    if kind == "i" or kind == "r":
        return val
    if kind == "e":
        return f"{ty}'val({val})" if re.fullmatch(r"[\w.]+", ty) else None
    if kind == "s":
        return '"' + val.replace('"', '""') + '"'
    if kind == "b":
        return f'"{val}"'
    return None


LOGIC = "UX01ZWLH-"


def testbench(desc, context, gens, ports, stim, std="2008"):
    """The replay testbench: the entity alone, its generics as recorded, its in ports driven from `stim` per delta cycle."""
    consts, gmap = [], []
    for n, ty, default in gens:
        k, val = desc["gens"].get(n.lower(), ("?", ""))
        lit = literal(k, val, ty) if k != "?" else None
        lit = lit or default
        if lit is None:
            raise Fail(f"generic {n} of {desc['entity']}: the recorder could not read its value and it has no default")
        consts.append(f"  constant {n} : {ty} := {lit};")
        gmap.append(f"{n} => {n}")
    sigs, pmap = [], []
    for n, mode, sub in ports:
        if mode == "in":
            sigs.append(f"  signal rp_{n} : {sub};")
            pmap.append(f"{n} => rp_{n}")
        elif mode in ("out", "buffer"):
            pmap.append(f"{n} => open")
        else:
            raise Fail(f"port {n} of {desc['entity']} is {mode}: the replay drives in ports only")
    var, case = [], []
    for i, k, name, ty in desc["leaves"]:
        s = f"a{i}"  # an object alias per leaf: `x(0).f'subtype` in a subtype indication is rejected by the parser (#1067)
        var.append(f"    alias {s} is rp_{name};")
        if k == "l":
            case.append(f"          when {i} => read(l, c); {s} <= rp_ul(c);")
        elif k == "i":
            case.append(f"          when {i} => read(l, d); {s} <= d;")
        elif std < "2008":  # VHDL-93 has no 'subtype: type marks from the recorder, vectors element by element
            if k == "e" and ty != "?":
                case.append(f"          when {i} => read(l, d); {s} <= {ty}'val(d);")
            elif k in "LI":
                elem = "read(l, c); {s}(j) <= rp_ul(c);" if k == "L" else "read(l, d); {s}(j) <= d;"
                case.append(f"          when {i} => for j in {s}'range loop {elem.format(s=s)} end loop;")
            else:
                raise Fail(f"port element {name} of {desc['entity']}: a VHDL-93 replay cannot drive a {'vector of enumerations' if k == 'E' else 'enumeration of unknown type'}")
        elif k == "e":
            case.append(f"          when {i} => read(l, d); {s} <= {s}'subtype'val(d);")
        else:
            var.append(f"    variable v{i} : {s}'subtype;")
            if k == "L" and ty in ("std_ulogic_vector", "std_logic_vector"):  # the textio read of the 2008 package
                case.append(f"          when {i} => read(l, v{i}); {s} <= v{i};")
                continue
            elem = {"L": "read(l, c); v{i}(j) := rp_ul(c);", "E": "read(l, d); v{i}(j) := v{i}(j)'subtype'val(d);", "I": "read(l, d); v{i}(j) := d;"}[k]
            case.append(f"          when {i} => for j in v{i}'range loop {elem.format(i=i)} end loop; {s} <= v{i};")
    logic = "\n".join(f"      when '{ch}' => return '{ch}';" for ch in LOGIC[:-1])
    nl = "\n"
    return f"""-- Generated by tools/realbench/replay.py: the in ports of {desc['inst']} replayed from {Path(stim).name}.
library ieee;
use ieee.std_logic_1164.all;
{nl.join(context)}
use std.textio.all;

entity replay_tb is
end entity;

architecture replay of replay_tb is
{nl.join(consts)}
{nl.join(sigs)}
  function rp_ul(c : character) return std_ulogic is
  begin
    case c is
{logic}
      when others => return '-';
    end case;
  end function;
begin
  dut : entity {desc['lib']}.{desc['entity']}({desc['arch']})
    generic map ({', '.join(gmap)})
    port map ({', '.join(pmap)});

  drive : process
    file f : text open read_mode is "{stim}";
    variable l : line;
    variable c : character;
    variable n, d, cur : integer;
    variable t : time;
{nl.join(var)}
  begin
    cur := -1;
    while not endfile(f) loop
      readline(f, l);
      if l.all(1) = 'T' then
        read(l, c); read(l, t);
        if t > now then
          wait for t - now;
        end if;
        cur := 0;
      elsif l.all(1) = 'D' then
        read(l, c); read(l, d);
        while cur < d loop
          wait for 0 ns;
          cur := cur + 1;
        end loop;
      else
        read(l, n); read(l, c);
        case n is
{nl.join(case)}
          when others => report "replay: no leaf " & integer'image(n) severity failure;
        end case;
      end if;
    end loop;
    wait;
  end process;
end architecture;
"""


# ---- runs ----

def run(cmd, what, cwd=None, env=None, ok=(0,), timeout=1800):
    p = subprocess.run(["timeout", str(timeout), "systemd-run", "--user", "--scope", "-q", "-p", f"MemoryMax={MEM}"] + cmd, cwd=cwd, env=env,
                       capture_output=True, text=True, errors="replace")
    if p.returncode not in ok:
        raise Fail(f"{what} failed (exit {p.returncode}): {(p.stdout + p.stderr).strip()[-800:]}")
    return p


def plugin(dest):
    src = HERE / "replay" / "record.c"
    so = dest / f"record-{hashlib.sha256(src.read_bytes()).hexdigest()[:12]}.so"
    if not so.exists():
        inc = R.REPO / "crates/yavs-vhpi/include"
        run(["cc", "-O2", "-shared", "-fPIC", "-I", str(inc), str(src), "-o", str(so)], "compiling the recorder plugin (needs a C compiler)")
    return so


def lib_of(world, path):
    """(library, std) of the world library that holds source file `path`."""
    for lib in R.libs_of(world):
        man = Path(world) / lib / "yavs-lib.toml"
        if man.exists():
            for f in tomllib.loads(man.read_text()).get("file", []):
                if f["path"] == path:
                    return lib, f.get("std", "2008")
    raise Fail(f"{path}: no library of the world holds it")


def yavs_cmd(Y, t, world, tier, top_lib, top, stop, gens=True, extra=()):
    a = [Y, "run", f"--std={t['std']}"] + (["--relaxed"] if t["relaxed"] else []) + [f"--work={top_lib}:{world}/{top_lib}"]
    a += [f"--map={l}:{world}/{l}" for l in R.libs_of(world) if l != top_lib]
    a += [f"--tier={tier}", f"--stop-time={stop}"] + list(extra) + [top] + (R.gen_args(t, world) if gens else [])
    return a


def record(Y, t, inst, until, pristine, store):
    """The stimulus of `inst` up to `until`, made once per test entry, image and instance (values do not depend on the yavs build)."""
    rec_src = (HERE / "replay" / "record.c").read_bytes()  # a new recorder writes another format
    key = hashlib.sha256(repr((t["name"], inst, until, rec_src, sorted((k, str(v)) for k, v in t.items() if k not in ("order",)))).encode()).hexdigest()[:12]
    stim = store / f"stim-{key}.txt"
    if stim.exists() and Path(str(stim) + ".ports").exists():
        return stim
    w = store / f"rec-world-{os.getpid()}"
    R.copy_tree(pristine, w)
    try:
        env = {**os.environ, "YAVS_REPLAY_INST": inst, "YAVS_REPLAY_OUT": str(stim) + ".tmp"}
        cmd = ["taskset", "-c", UNTIMED] + yavs_cmd(Y, t, w, "interp", t["toplib"], t["top"], until, extra=[f"--vhpi={plugin(store)}"])
        t0 = time.time()
        run(["bash", "-c", f"cd {shlex.quote(str(w))} && exec \"$@\" >rec.out 2>rec.err || {{ s=$?; tail -c 600 rec.out rec.err; exit $s; }}", "_"] + cmd,
            f"recording {inst}", env=env, ok=(0, t["exit_code"]) if t["exit_code"] else (0,))
        os.replace(str(stim) + ".tmp.ports", str(stim) + ".ports")
        os.replace(str(stim) + ".tmp", stim)
        print(f"recorded {inst} to {until} in {time.time() - t0:.0f} s: {stim.stat().st_size / 1e6:.0f} MB", flush=True)
    finally:
        shutil.rmtree(w, ignore_errors=True)
    return stim


def replay_world(Y, t, stim, pristine, dest):
    """A copy of the test's world with the replay testbench analysed into the entity's library; returns (library, world)."""
    desc = parse_ports_file(Path(str(stim) + ".ports").read_text())
    lib, std = lib_of(pristine, desc["file"])
    if desc["lib"] == "?":
        desc["lib"] = lib
    context, gens, ports = entity_header(Path(desc["file"]).read_text(errors="replace"), desc["entity"])
    R.copy_tree(pristine, dest)
    tb = dest / "replay_tb.vhd"
    tb.write_text(testbench(desc, context, gens, ports, str(stim), std))
    flags = ["--relaxed"] if t["relaxed"] else []
    # the entity's standard: its packages may not analyse under another (GRLIB at 2008)
    a = [Y, "analyze", f"--std={std}"] + flags + [f"--work={lib}:{dest}/{lib}"] + [f"--map={l}:{dest}/{l}" for l in R.libs_of(dest) if l != lib] + [str(tb)]
    run(["taskset", "-c", UNTIMED] + a, f"analysing the replay testbench {tb}", cwd=dest)
    return lib, desc


def perf_stat(cmd, world, core, pf):
    script = f"cd {shlex.quote(str(world))} && " + W.perf_prefix(pf, False) + " ".join(shlex.quote(x) for x in cmd) + " >run.out 2>run.err"
    t0 = time.time()
    p = run(["taskset", "-c", str(core), "bash", "-c", script], "the timed replay", ok=(0, 1))
    res = {"wall": time.time() - t0, "rc": p.returncode}
    res.update(W.parse_perf(pf.read_text()))
    return res


def perf_record(cmd, world, core, tag, ok):
    """Samples per (event, symbol) of one run under perf record, with the perf map of the run (YAVS_CLIFT_PERFMAP) read and removed."""
    data = world / f"{tag}.data"
    t0 = time.time()
    for pages in (64, 16, 8, 4):  # the locked-page budget of perf buffers is per user, shared by every agent: a busy host needs a small one
        data.unlink(missing_ok=True)
        rec = ["perf", "record", "-q", "--no-inherit", "-m", str(pages), "-o", str(data)] + sum([["-e", f"{e}/period={p}/u"] for e, p in PERIOD.items()], []) + ["--"] + cmd
        script = f"cd {shlex.quote(str(world))} && YAVS_CLIFT_PERFMAP=1 " + " ".join(shlex.quote(x) for x in rec) + f" >{tag}.out 2>{tag}.err"
        t0 = time.time()
        p = run(["taskset", "-c", str(core), "bash", "-c", script], f"perf record of the {tag} run", ok=range(256))  # status checked after the map is removed
        p_run = p
        if p.returncode != 255 or (data.exists() and data.stat().st_size):
            break
    else:
        raise Fail("perf record could not map its buffer even with 4 pages (perf_event_mlock_kb is used up by other perf runs); try again later")
    wall = time.time() - t0
    counts, pids = defaultdict(int), set()
    sizes = {}
    p = subprocess.Popen(["systemd-run", "--user", "--scope", "-q", "-p", f"MemoryMax={MEM}", "perf", "script", "-i", str(data), "-F", "pid,event,ip,sym"],
                         stdout=subprocess.PIPE, stderr=subprocess.DEVNULL, text=True, errors="replace")
    for line in p.stdout:  # streamed: a full run gives millions of lines
        f = line.split(None, 3)  # pid, event, ip, symbol (which can hold spaces)
        if len(f) >= 3:
            pids.add(f[0])
            counts[(f[1].split("/")[0], f[3].strip() if len(f) > 3 else "?")] += 1  # event `cycles/period=N/u:`
    p.wait()
    for pid in pids:  # the perf map of the recorded process (its pid is in the samples): read, then removed
        m = Path(f"/tmp/perf-{pid}.map")
        if m.exists():
            for line in m.read_text().splitlines():
                a, s, name = line.split(" ", 2)
                sizes[name] = int(s, 16)
            m.unlink()
    data.unlink(missing_ok=True)
    if p_run.returncode not in ok:
        raise Fail(f"perf record of the {tag} run failed (exit {p_run.returncode}): {(world / f'{tag}.err').read_text(errors='replace')[-600:]}")
    return {"counts": counts, "sizes": sizes, "wall": wall}


def symbol_rows(rec, label):
    """Per compiled function of the process (`f<id>_<label>`, `n<id>_<label>`): its code size and the counts of each event."""
    rows = {}
    for (ev, sym), n in rec["counts"].items():
        if re.fullmatch(rf"[fn]\d+_{re.escape(label)}", sym):
            rows.setdefault(sym, {"size": rec["sizes"].get(sym)})[ev] = n * PERIOD[ev]
    return rows


def coverage(Y, cmd, world, prefix, ok):
    """Hit counts of the statements and branches under instance `prefix`: (kind, path inside it, file, line, column, text) -> hits."""
    run(["taskset", "-c", UNTIMED, "bash", "-c", f"cd {shlex.quote(str(world))} && exec \"$@\" >/dev/null 2>&1", "_"] + cmd, f"the coverage run in {world.name}", ok=ok)
    db = world / "cov.ycov"
    run([Y, "cov", "report", str(db), "--json", f"{db}.json"], "the coverage report")
    out = {}
    for it in json.loads(Path(f"{db}.json").read_text())["items"]:
        path = it["instance"].lower()
        if path == prefix or path.startswith(prefix + "."):
            k = (it["kind"], path[len(prefix):], Path(it["file"]).name, it["line"], it.get("column"), it.get("statement") or it.get("branch") or "")
            out[k] = out.get(k, 0) + it["hits"]
    Path(f"{db}.json").unlink()
    db.unlink()
    return out


def profile_secs(path, prefix, label):
    """(seconds, compiled share) the profile saw process `label` of instance `prefix` run; (None, None) without the file."""
    if not Path(path).exists():
        return None, None
    d = json.loads(Path(path).read_text())
    ps = [p for p in d["processes"] if p["path"].lower() == f"{prefix}.{label}".lower()]
    n = sum(p["samples"] for p in ps)
    comp = sum(x["samples"] for p in ps for x in p.get("subprograms", []) if x["compiled"])
    return n / d["hz"], comp / n if n else None


def matching(full_rows, rep_rows):
    """The full run's functions of the process that have the replay's code sizes (other entities can have processes with the same label),
    and whether a size matched more functions than the replay has."""
    want = [r["size"] for r in rep_rows.values()]
    if not want:
        return full_rows, False
    picked = {k: r for k, r in full_rows.items() if r["size"] in want}
    return picked, any(sum(r["size"] == s for r in picked.values()) > want.count(s) for s in set(want))


# Layers of a run by symbol, for the shares where cycles go (perf script names; Rust paths demangled)
LAYERS = (
    ("seam: gather, scatter", re.compile(r"Machine>::(sig_gather|scatter_leaves|sig_scatter)\b|rt::.*(gather|scatter)")),
    ("signal update", re.compile(r"yavs_kernel::signal::")),
    ("kernel cycle", re.compile(r"yavs_kernel::")),
    ("generated code", re.compile(r"^[fn]\d+_|^\[unknown\]$")),
    ("rt callbacks", re.compile(r"yavs_rt::|yavs_clift::|yavs_rt_")),
    ("interpreter", re.compile(r"yavs_interp::")),
)
FOCUS = ("scatter_leaves", "direct_same_planes", "sig_gather", "commit_bytes")


def layers(counts, event="cycles"):
    """Shares (percent) of `event` per layer, plus the functions in FOCUS, from perf_record counts."""
    tot = sum(n for (e, _), n in counts.items() if e == event) or 1
    out = defaultdict(float)
    for (e, sym), n in counts.items():
        if e != event:
            continue
        name = next((ln for ln, rx in LAYERS if rx.search(sym)), "other (libc, std, perf)")
        out[name] += 100 * n / tot
        for f in FOCUS:
            if re.search(rf"::{f}\b", sym):
                out[f] += 100 * n / tot
    return out


def layer_table(cols):
    """Text table of layers: cols is [(name, layers dict)]."""
    names = [n for n, _ in LAYERS] + ["other (libc, std, perf)"] + list(FOCUS)
    lines = [f"  {'cycles, percent':24}" + "".join(f"{n:>12}" for n, _ in cols)]
    for k in names:
        lines.append(f"  {k:24}" + "".join(f"{d.get(k, 0):12.1f}" for _, d in cols))
    return "\n".join(lines)


def summarize(rows):
    """Totals over the process's symbols, plus the rates."""
    tot = {e: sum(r.get(e, 0) for r in rows.values()) for e in PERIOD}
    ins = tot["instructions"]
    return {"ins": ins, "cyc": tot["cycles"], "ipc": ins / tot["cycles"] if tot["cycles"] else None,
            "l1i": 1e3 * tot["L1-icache-load-misses"] / ins if ins else None, "br": 1e3 * tot["branch-misses"] / ins if ins else None,
            "kinds": sorted(re.sub(r"\d+_.*", "", s) for s in rows), "sizes": sorted(r["size"] or 0 for r in rows.values())}


def wait_core(want):
    """A timing core, waiting up to an hour for one to come free (other agents' timed runs hold them)."""
    for i in range(120):
        try:
            return W.take_core(want)
        except W.Fail:  # window.py's own copy of run.py: its Fail is another class
            if i == 0:
                print("waiting for a free timing core (tools/claim core)", flush=True)
            time.sleep(30)
    return W.take_core(want)


def fmt(x, spec):
    return "-" if x is None else format(x, spec)


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("bin", nargs="?", help="yavs binary (tools/build-bench.sh --features clift)")
    ap.add_argument("target", nargs="?", help="one of: " + ", ".join(TARGETS))
    ap.add_argument("--tier", default="auto")
    ap.add_argument("--until", help="end of the recording and of both runs (default: the target's)")
    ap.add_argument("--core", type=int, help="timing core, default a free one of 9-15")
    ap.add_argument("--check", action="store_true", help="also run the full design under perf record and compare the process's own symbols")
    ap.add_argument("--keep", action="store_true", help="keep the scratch directory (the replay world with replay_tb.vhd) and print its path")
    ap.add_argument("--list", action="store_true")
    ap.add_argument("--self-test", action="store_true")
    a = ap.parse_args()
    if a.self_test:
        return self_test(a.bin)
    if a.list or not a.target:
        for n, (test, inst, label, until) in TARGETS.items():
            print(f"{n:12} {test:20} {label:12} to {until:6} {inst}")
        return 0
    if a.target not in TARGETS:
        raise Fail(f"unknown target `{a.target}`; known: {', '.join(TARGETS)}")
    test, inst, label, until = TARGETS[a.target]
    until = a.until or until
    Y = str(Path(a.bin).expanduser().resolve())
    if not Path(Y).is_file():
        raise Fail(f"binary {Y} not found; build it with tools/build-bench.sh --features clift")
    t = {x["name"]: x for x in R.load_tests()}[test]
    R.check_sources([t])
    os.sched_setaffinity(0, W.SETUP_CPUS & os.sched_getaffinity(0) or os.sched_getaffinity(0))
    store = R.CACHE / "replay"
    store.mkdir(parents=True, exist_ok=True)
    scratch = R.CACHE / "scratch" / f"replay-{os.getpid()}"
    scratch.mkdir(parents=True, exist_ok=True)
    try:
        if t["image"]:
            t["image_dir"] = str(R.P.build(t["image"]))
        key = R.B.binary_key(Y)
        for m in t["corpus"]:
            R.ensure_base("yavs", m, Y, key)
        pristine = R.make_pristine(t, "yavs", Y, key, scratch / "pristine")
        stim = record(Y, t, inst, until, pristine, store)
        lib, desc = replay_world(Y, t, stim, pristine, scratch / "replay")
        rw = scratch / "replay"
        rcmd = yavs_cmd(Y, t, rw, a.tier, lib, "replay_tb", until, gens=False)
        for _ in range(2):  # warm: the first run leaves a note in the cache, the second makes the entries
            run(["taskset", "-c", UNTIMED] + rcmd, "a replay warm-up", cwd=rw)
        if a.check:  # the untimed part of the check first, so the timing core is held only for timed runs
            fw = scratch / "full"
            R.copy_tree(pristine, fw)
            ok = (0, t["exit_code"]) if t["exit_code"] else (0,)
            prefixes = (inst.lower(), "replay_tb.dut")
            # 1. wakes and paths: statement and branch hit counts of every process of the instance (interpreter, coverage on)
            cov = [coverage(Y, yavs_cmd(Y, t, w, "interp", tl, top, until, gens=g, extra=["--cov=sb", f"--cov-file={w}/cov.ycov"]), w, pre, okc)
                   for w, tl, top, g, pre, okc in ((fw, t["toplib"], t["top"], True, prefixes[0], ok), (rw, lib, "replay_tb", False, prefixes[1], (0, 1)))]
            fcmd = yavs_cmd(Y, t, fw, a.tier, t["toplib"], t["top"], until)
            for _ in range(2):
                run(["taskset", "-c", UNTIMED, "bash", "-c", f"cd {shlex.quote(str(fw))} && exec \"$@\" >/dev/null 2>&1", "_"] + fcmd, "a full-run warm-up", ok=ok)
        core, _lock = wait_core(a.core)
        st = perf_stat(rcmd, rw, core, scratch / "replay.perf")
        rr = W.rates(st)
        print(f"\nreplay of {label} in {desc['entity']} ({inst}), {a.tier}, to {until}, core {core}, load {os.getloadavg()[0]:.1f}")
        print(f"  whole replay: wall {st['wall']:.1f} s, instructions {st['instructions'] / 1e9:.2f} G, cycles {st['cycles'] / 1e9:.2f} G, IPC {fmt(rr.get('ipc'), '.2f')}, "
              f"L1i/k {fmt(rr.get('l1i'), '.2f')}, br/k {fmt(rr.get('branch'), '.2f')}")
        if not a.check:
            return 0
        keys = set(cov[0]) | set(cov[1])
        same = sum(cov[0].get(k) == cov[1].get(k) for k in keys)
        worst = max((abs((cov[0].get(k) or 0) - (cov[1].get(k) or 0)) for k in keys), default=0)
        print(f"\ncoverage of the instance, full run against replay: {same} of {len(keys)} statement and branch hit counts equal, the others differ by at most {worst}")
        # 2. the process's own code: perf samples by symbol, and the profile's samples of the process
        rcmd_p = yavs_cmd(Y, t, rw, a.tier, lib, "replay_tb", until, gens=False, extra=[f"--profile={rw}/prof.json"])
        fcmd_p = yavs_cmd(Y, t, fw, a.tier, t["toplib"], t["top"], until, extra=[f"--profile={fw}/prof.json"])
        for w, c, okc in ((rw, rcmd_p, (0, 1)), (fw, fcmd_p, ok)):  # profile runs of their own: perf record slows a run and its profile
            run(["taskset", "-c", str(core), "bash", "-c", f"cd {shlex.quote(str(w))} && exec \"$@\" >/dev/null 2>&1", "_"] + c, "a profile run", ok=okc)
        # perf record on cold copies: code linked from the compilation cache gets no perf map entry, so a warm run has no symbols for it
        cold = []
        for w in (rw, fw):
            c = w.with_name(w.name + "-cold")
            R.copy_tree(w, c)
            for d in c.glob("*/yavs-cache"):
                shutil.rmtree(d)
            cold.append(c)
        rep = perf_record([x.replace(str(rw), str(cold[0])) for x in rcmd], cold[0], core, "replay", (0, 1))
        full = perf_record([x.replace(str(fw), str(cold[1])) for x in fcmd], cold[1], core, "full", ok)
        # the layers from warm runs, as measured runs are (their generated code has no symbol: `[unknown]`)
        lay = [layers(perf_record(c, w, core, tag, k)["counts"]) for c, w, tag, k in ((rcmd, rw, "replay-warm", (0, 1)), (fcmd, fw, "full-warm", ok))]
        secs = [profile_secs(w / "prof.json", pre, label) for w, pre in ((rw, prefixes[1]), (fw, prefixes[0]))]
        a_rows = symbol_rows(rep, label)
        b_rows, ambiguous = matching(symbol_rows(full, label), a_rows)
        print(f"\n{label}, own code (perf samples times period; replay {rep['wall']:.1f} s, full run {full['wall']:.1f} s, both under perf record on a cold cache; profile seconds from warm runs without it)")
        print(f"  {'':7} {'profile s':>9} {'compiled':>8} {'functions':10} {'code bytes':14} {'instr G':>8} {'cycles G':>8} {'IPC':>5} {'L1i/k':>6} {'br/k':>6}")
        sa, sb = summarize(a_rows), summarize(b_rows)
        for n, s_, (sec, comp) in (("replay", sa, secs[0]), ("full", sb, secs[1])):
            print(f"  {n:7} {fmt(sec, '9.2f')} {fmt(comp and 100 * comp, '7.0f')}% {','.join(s_['kinds']) or 'none':10} {','.join(map(str, s_['sizes'])) or '-':14} {s_['ins'] / 1e9:8.3f} {s_['cyc'] / 1e9:8.3f} "
                  f"{fmt(s_['ipc'], '5.2f')} {fmt(s_['l1i'], '6.2f')} {fmt(s_['br'], '6.2f')}")
        q = lambda x, y: x / y if x and y else None
        print(f"  {'ratio':7} {fmt(q(secs[0][0], secs[1][0]), '9.3f')} {'':8}  {'':10} {'':14} {fmt(q(sa['ins'], sb['ins']), '8.3f')} {fmt(q(sa['cyc'], sb['cyc']), '8.3f')} "
              f"{fmt(q(sa['ipc'], sb['ipc']), '5.2f')} {fmt(q(sa['l1i'], sb['l1i']), '6.2f')} {fmt(q(sa['br'], sb['br']), '6.2f')}")
        print(f"\nwhere the cycles of each run go, by layer (warm runs: whole replay against the whole full run)\n{layer_table([('replay', lay[0]), ('full', lay[1])])}")
        if not a_rows and not b_rows:
            print(f"  {label} has no compiled code in either cold run at --tier={a.tier}: it ran in the interpreter, which perf cannot split by process; "
                  "the profile columns (time in the process, share compiled, warm runs) and the coverage line are the per-process check. --tier=clift compiles it in both")
        elif sa["sizes"] != sb["sizes"]:
            print("  CODE DIFFERS: the replay compiled other functions or sizes than the full run, so it does not stand in for it")
        if ambiguous:
            print(f"  note: other processes named {label} in the full run have code of the same size; their samples are counted with it")
        return 0
    finally:
        if a.keep:
            print(f"kept {scratch}")
        else:
            shutil.rmtree(scratch, ignore_errors=True)


def self_test(Y=None):
    src = """library ieee; use ieee.std_logic_1164.all;
use work.p.all; -- comment ; with a semicolon
entity dut is
  generic (W : natural := 4; G, H : boolean := false; S : string := "a--b");
  port (clk : in std_ulogic; a : in arr_t; c : in std_ulogic_vector(W-1 downto 0) := (others => '0');
        q : out unsigned(3 downto 0); signal z : buffer bit);
end entity;
architecture rtl of dut is begin end;
"""
    ctx, gens, ports = entity_header(src, "DUT")
    assert ctx == ["library ieee;", "use ieee.std_logic_1164.all;", "use work.p.all;"], ctx
    assert gens == [("W", "natural", "4"), ("G", "boolean", "false"), ("H", "boolean", "false"), ("S", "string", '"a--b"')], gens
    assert ports == [("clk", "in", "std_ulogic"), ("a", "in", "arr_t"), ("c", "in", "std_ulogic_vector(W-1 downto 0)"), ("q", "out", "unsigned(3 downto 0)"),
                     ("z", "buffer", "bit")], ports
    desc = parse_ports_file("I :TB:U\nE ? DUT RTL\nF /x/d.vhd\nG W i 3\nG G e 1\nG S s hi\nP CLK in\nL 0 l CLK STD_ULOGIC\nP A in\nL 1 e A(0).X E_T\nL 2 L A(0).V UNSIGNED\nL 3 I A(1).N ?\n"
                            "P C in\nL 4 L C STD_ULOGIC_VECTOR\nP Q out\nP Z buffer\n")
    assert desc["leaves"][1] == (1, "e", "a(0).x", "e_t") and desc["gens"]["g"] == ("e", "1") and desc["lib"] == "?"
    desc["lib"] = "work"
    tb = testbench(desc, ctx, gens, ports, "/s/stim.txt")
    for want in ("constant W : natural := 3;", "constant G : boolean := boolean'val(1);", "constant H : boolean := false;", 'constant S : string := "hi";',
                 "signal rp_c : std_ulogic_vector(W-1 downto 0);", "q => open", "z => open", "entity work.DUT(RTL)",
                 "alias a1 is rp_a(0).x;", "when 1 => read(l, d); a1 <= a1'subtype'val(d);", "variable v2 : a2'subtype;", "when 3 => for j in v3'range loop read(l, d); v3(j) := d; end loop;",
                 "when 2 => for j in v2'range loop read(l, c); v2(j) := rp_ul(c); end loop;", "when 4 => read(l, v4); a4 <= v4;"):
        assert want in tb, (want, tb)
    tb93 = testbench(desc, ctx, gens, ports, "/s/stim.txt", "1993")
    for want in ("when 1 => read(l, d); a1 <= e_t'val(d);", "when 2 => for j in a2'range loop read(l, c); a2(j) <= rp_ul(c); end loop;"):
        assert want in tb93, (want, tb93)
    assert "'subtype" not in tb93
    try:
        testbench(desc, ctx, gens, ports + [("io", "inout", "std_logic")], "/s")
        raise AssertionError("an inout port passed")
    except Fail as e:
        assert "inout" in str(e)
    assert set(TARGETS) == {"execute1_1", "execute1_dp", "decode2_1", "dctrl", "iu"} and all(v[0] in {x["name"] for x in R.load_tests()} for v in TARGETS.values())
    rows = symbol_rows({"counts": {("instructions", "f12_comb"): 3, ("cycles", "n13_comb"): 2, ("instructions", "f1_combx"): 9}, "sizes": {"f12_comb": 64}}, "comb")
    assert set(rows) == {"f12_comb", "n13_comb"} and rows["f12_comb"]["size"] == 64 and rows["f12_comb"]["instructions"] == 3 * PERIOD["instructions"]
    if Y:
        end_to_end(str(Path(Y).expanduser().resolve()))
    ly = layers({("cycles", "<yavs_interp::exec::Machine>::scatter_leaves"): 3, ("cycles", "<yavs_kernel::signal::Signal>::direct_same_planes"): 1,
                 ("cycles", "f3_comb"): 4, ("instructions", "f3_comb"): 9})
    assert ly["seam: gather, scatter"] == 37.5 and ly["scatter_leaves"] == 37.5 and ly["signal update"] == 12.5 and ly["generated code"] == 50, ly
    print("replay self-test: ok" + (" (with a recording and a replay)" if Y else ""))
    return 0


E2E = """library ieee; use ieee.std_logic_1164.all; use ieee.numeric_std.all;
package p is
  type e_t is (aa, bb, cc);
  type r_t is record x : e_t; v : unsigned(3 downto 0); n : integer range 0 to 100; s : std_ulogic_vector(1 downto 0); end record;
  type arr_t is array (0 to 1) of r_t;
end;
library ieee; use ieee.std_logic_1164.all; use ieee.numeric_std.all; use work.p.all;
entity dut is
  generic (W : natural := 4; G : boolean := false);
  port (clk : in std_ulogic; a : in arr_t; c : in std_ulogic_vector(W-1 downto 0); q : out unsigned(3 downto 0));
end;
architecture rtl of dut is
  signal acc : unsigned(3 downto 0) := (others => '0');
begin
  process (clk) begin
    if rising_edge(clk) then acc <= acc + a(0).v + a(1).n mod 16 + e_t'pos(a(1).x); end if;
  end process;
  q <= acc;
  process (acc, c, a) begin report to_string(acc) & " " & to_string(c) & " " & to_string(a(1).s) & " " & boolean'image(G); end process;
end;
library ieee; use ieee.std_logic_1164.all; use ieee.numeric_std.all; use work.p.all;
entity tb is end;
architecture t of tb is
  signal clk : std_ulogic := '0';
  signal a : arr_t := (others => (aa, "0000", 0, "00"));
  signal c, c2 : std_ulogic_vector(2 downto 0) := "000";
begin
  clk <= not clk after 5 ns;
  process (clk) begin
    if rising_edge(clk) then
      a(0).v <= a(0).v + 1; a(1).n <= (a(1).n + 3) mod 100; c2 <= std_ulogic_vector(unsigned(c2) + 1);
      a(1).x <= e_t'val((e_t'pos(a(1).x) + 1) mod 3); a(1).s <= not a(1).s;
    end if;
  end process;
  c <= c2;  -- one delta after c2
  u : entity work.dut generic map (W => 3, G => true) port map (clk => clk, a => a, c => c, q => open);
end;
"""


def end_to_end(Y):
    """Record a small design, replay its instance, and compare what the instance reports: the same values at the same times, one delta later."""
    d = R.CACHE / "scratch" / f"replay-selftest-{os.getpid()}"
    shutil.rmtree(d, ignore_errors=True)
    d.mkdir(parents=True)
    try:
        (d / "d.vhd").write_text(E2E)
        run([Y, "analyze", "--std=2008", f"--work=work:{d}/w/work", str(d / "d.vhd")], "analysing the self-test design")
        stim = d / "stim.txt"
        env = {**os.environ, "YAVS_REPLAY_INST": "tb.u", "YAVS_REPLAY_OUT": str(stim)}
        full = run([Y, "run", "--std=2008", f"--work=work:{d}/w/work", f"--vhpi={plugin(d)}", "--stop-time=60ns", "tb"], "the recording run", env=env).stdout
        lib, _ = replay_world(Y, {"relaxed": False}, stim, d / "w", d / "rw")  # the path the targets take: library lookup, header, analysis
        rep = run([Y, "run", "--std=2008", f"--work={lib}:{d}/rw/{lib}", "--stop-time=60ns", "replay_tb"], "the replay").stdout
        note = re.compile(r"^\*\* Note: (\d+)fs\+(\d+): (.*)$", re.M)
        a, b = note.findall(full), note.findall(rep)
        assert b[0][0] == "0" and b[0][2].startswith("0000 UUU"), b[:2]  # time 0: the ports start at their defaults, the recorded start one delta later
        a, b = [x for x in a if x[0] != "0"], [x for x in b if x[0] != "0"]
        assert len(a) == len(b) > 5 and all((t1, m1) == (t2, m2) and int(d2) == int(d1) + 1 for (t1, d1, m1), (t2, d2, m2) in zip(a, b)), (a, b)
    finally:
        shutil.rmtree(d, ignore_errors=True)


if __name__ == "__main__":
    try:
        sys.exit(main())
    except (Fail, W.Fail, R.P.Fail) as e:
        sys.exit(f"replay: {e}")

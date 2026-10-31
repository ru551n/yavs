#!/usr/bin/env python3
"""Program images of the cpubench workloads (tools/realbench/tests.toml, `image = { program = ..., ... }`).

The sources are read from the corpus cache at the pins of tools/fetch-corpus.sh (nothing is copied into the repository); the program is
built once into <cache>/images/<program>-<key>/ and reused, where the key covers the pinned commits, the parameters, the compiler
and RECIPE. Cross compilers are never installed by this tool: they are found through the environment or PATH, and a missing one stops
the test with what is needed.

  programs.py PROGRAM [key=value ...]     build (or find) the image, print its directory

Programs:
  microwatt_coremark  EEMBC CoreMark on Microwatt (iterations=N), own port in cpu/microwatt/coremark, as main_ram.hex (needs a POWER little-endian
                    bare-metal compiler: $YAVS_PPC_PREFIX = the tool prefix of a GCC (powerpc64le-linux-gnu-), or $YAVS_PPC_ZIG = a zig 0.14+
                    binary (its bundled clang and lld; the ziglang wheel), or powerpc64le-linux-gnu-gcc on PATH)
  neorv32_coremark  EEMBC CoreMark on the NEORV32 port (iterations=N), as neorv32_imem_image.vhd (needs a RISC-V GCC:
                    $YAVS_RISCV_PREFIX = the tool prefix, e.g. /opt/riscv/bin/riscv-none-elf-, or riscv-none-elf-gcc,
                    riscv64-unknown-elf-gcc or riscv32-unknown-elf-gcc on PATH)
Python is stdlib only; run with `python3 -I`.
"""
import hashlib, os, re, shlex, shutil, subprocess, sys
from pathlib import Path

HERE = Path(__file__).resolve().parent
RECIPE = 1  # bump when a build step changes: the stored images of earlier recipes are not reused
CACHE = Path(os.environ.get("YAVS_REALBENCH_CACHE", "~/.cache/yavs-realbench")).expanduser()
CORPUS = Path(os.environ.get("YAVS_CORPUS", "~/.cache/rsim-corpus")).expanduser()


class Fail(Exception):
    """A named, user-visible failure."""


def head(sid):
    root = CORPUS / sid
    p = subprocess.run(["git", "-C", str(root), "rev-parse", "HEAD"], capture_output=True, text=True)
    if p.returncode:
        raise Fail(f"source checkout `{sid}` not found at {root} (tools/fetch-corpus.sh)")
    return p.stdout.strip()


def run(argv, cwd, what):
    p = subprocess.run([str(a) for a in argv], cwd=cwd, capture_output=True, text=True, errors="replace")
    if p.returncode:
        raise Fail(f"{what} failed (exit {p.returncode}): {' '.join(str(a) for a in argv)}\n" + (p.stdout + p.stderr)[-1500:])
    return p.stdout


def riscv_prefix():
    """The RISC-V GCC tool prefix: $YAVS_RISCV_PREFIX, else the first of the usual prefixes with a gcc on PATH."""
    if os.environ.get("YAVS_RISCV_PREFIX"):
        pre = os.environ["YAVS_RISCV_PREFIX"]
        if not shutil.which(pre + "gcc"):
            raise Fail(f"$YAVS_RISCV_PREFIX={pre}: {pre}gcc not found")
        return pre
    for pre in ("riscv-none-elf-", "riscv64-unknown-elf-", "riscv32-unknown-elf-"):
        if shutil.which(pre + "gcc"):
            return pre
    raise Fail("no RISC-V GCC found. The neorv32_coremark image needs a bare-metal rv32 GCC (xPack riscv-none-elf-gcc, or riscv64-unknown-elf-gcc "
               "with rv32 multilibs): put its bin directory on PATH or set YAVS_RISCV_PREFIX to the tool prefix (for example /opt/xpack/bin/riscv-none-elf-)")


def stored(name, parts):
    """<cache>/images/<name>-<key>: the key hashes the recipe and everything that changes the image."""
    key = hashlib.sha256(repr((RECIPE, parts)).encode()).hexdigest()[:12]
    return CACHE / "images" / f"{name}-{key}"


def neorv32_coremark(iterations):
    pre = riscv_prefix()
    version = run([pre + "gcc", "--version"], None, "compiler version").splitlines()[0]
    d = stored("neorv32_coremark", [head("neorv32"), head("neorv32-coremark"), head("eembc-coremark"), iterations, version])
    if (d / "neorv32_imem_image.vhd").is_file():
        return d
    b = d.with_name(d.name + ".tmp")
    shutil.rmtree(b, ignore_errors=True)
    (b / "neorv32").mkdir(parents=True)
    for f in ("core_portme.c", "core_portme.h", "makefile"):
        shutil.copy(CORPUS / "neorv32-coremark" / f, b / f)
    shutil.copytree(CORPUS / "neorv32" / "sw", b / "neorv32" / "sw")  # the makefile builds the image generator inside it
    shutil.copytree(CORPUS / "eembc-coremark", b / "coremark", ignore=shutil.ignore_patterns(".git"))
    # The port fixes ITERATIONS unconditionally and the simulation UART mode is a compile flag: both go in on a copy, in the build
    # directory only (the pinned sources stay as fetched).
    h = (b / "core_portme.h").read_text()
    h2 = h.replace("#define ITERATIONS (2000)", "#ifndef ITERATIONS\n#define ITERATIONS (2000)\n#endif")
    m = (b / "makefile").read_text()
    m2 = m.replace("# Sources", f"USER_FLAGS += -DUART0_SIM_MODE -DITERATIONS={int(iterations)}\n\n# Sources")
    if h2 == h or m2 == m:
        raise Fail("neorv32_coremark: the port's core_portme.h or makefile no longer has the lines the build edits (programs.py, RECIPE)")
    (b / "core_portme.h").write_text(h2)
    (b / "makefile").write_text(m2)
    run(["make", "-s", f"RISCV_PREFIX={pre}", "image"], b, "building the neorv32_coremark image")
    img = b / "neorv32_imem_image.vhd"
    if not img.is_file():
        raise Fail(f"neorv32_coremark: the build left no {img}")
    size = run([pre + "size", b / "main.elf"], b, "size").splitlines()[-1].split()
    (b / "build.txt").write_text(f"{version}\niterations {iterations}\ntext {size[0]} data {size[1]} bss {size[2]}\n")
    d.parent.mkdir(parents=True, exist_ok=True)
    shutil.rmtree(d, ignore_errors=True)
    for junk in ("neorv32", "coremark", "build", "main.elf", "elf.bin", "core_portme.c", "core_portme.h", "makefile"):
        p = b / junk
        shutil.rmtree(p, ignore_errors=True) if p.is_dir() else p.unlink(missing_ok=True)
    b.rename(d)
    return d


def ppc_tools():
    """(compiler command, objcopy command, version) for POWER little-endian freestanding code, from the environment (see the module doc)."""
    if os.environ.get("YAVS_PPC_ZIG"):
        zig = os.environ["YAVS_PPC_ZIG"]
        if not Path(zig).is_file():
            raise Fail(f"$YAVS_PPC_ZIG={zig} is not a file")
        cc, oc = [zig, "cc", "-target", "powerpc64le-freestanding", "-mcpu=pwr8", "-mno-power8-vector", "-mno-direct-move", "-mno-crypto"], [zig, "objcopy"]
    else:
        pre = os.environ.get("YAVS_PPC_PREFIX") or next((x for x in ("powerpc64le-linux-gnu-", "powerpc64le-unknown-linux-gnu-") if shutil.which(x + "gcc")), None)
        if not pre or not shutil.which(pre + "gcc"):
            raise Fail("no POWER cross compiler found. The microwatt_coremark image needs a little-endian POWER (ppc64le) bare-metal compiler: a GCC "
                       "(Debian/Ubuntu package gcc-powerpc64le-linux-gnu: set YAVS_PPC_PREFIX=powerpc64le-linux-gnu- or put it on PATH), or a zig 0.14+ "
                       "binary (the PyPI ziglang wheel carries one): set YAVS_PPC_ZIG to it. Alternatively a ready-made main_ram.hex of CoreMark for "
                       "Microwatt would do, but the project publishes none")
        cc, oc = [pre + "gcc", "-mcpu=power8", "-msoft-float", "-mstrict-align"], [pre + "objcopy"]
    return cc, oc, run(cc + ["--version"], None, "compiler version").splitlines()[0]


def microwatt_coremark(iterations):
    cc, objcopy, version = ppc_tools()
    mw, em, port = CORPUS / "microwatt", CORPUS / "eembc-coremark", HERE / "cpu" / "microwatt" / "coremark"
    portsrc = [(port / f).read_bytes() for f in sorted(os.listdir(port))]
    d = stored("microwatt_coremark", [head("microwatt"), head("eembc-coremark"), iterations, version, hashlib.sha256(b"".join(portsrc)).hexdigest()])
    if (d / "main_ram.hex").is_file():
        return d
    b = d.with_name(d.name + ".tmp")
    shutil.rmtree(b, ignore_errors=True)
    (b / "cm").mkdir(parents=True)
    for f in ("core_list_join.c", "core_main.c", "core_matrix.c", "core_state.c", "core_util.c", "coremark.h"):
        shutil.copy(em / f, b / "cm" / f)
    # The barebones printf has an empty uart_send_char behind an #error (the port defines the real one): renamed away on the copy.
    pf = (em / "barebones" / "ee_printf.c").read_text()
    pf2 = re.sub(r'#error "You must implement the method uart_send_char[^\n]*\n', "", pf).replace("\nuart_send_char(char c)\n{", "\nuart_send_char_unused(char c)\n{")
    if pf2.count("uart_send_char_unused") != 1 or "#error" in pf2:
        raise Fail("microwatt_coremark: barebones/ee_printf.c no longer has the #error line the build removes (programs.py, RECIPE)")
    (b / "cm" / "ee_printf.c").write_text(pf2)
    for f in os.listdir(port):
        if f.endswith((".c", ".h")):
            shutil.copy(port / f, b / "cm" / f)
    shutil.copy(mw / "lib" / "console.c", b / "cm" / "console.c")
    shutil.copy(mw / "hello_world" / "head.S", b / "cm" / "head.S")
    flags = ["-O2", "-ffreestanding", "-fno-pic", "-fno-stack-protector", "-mno-vsx", "-mno-altivec", "-DPERFORMANCE_RUN=1",
             f"-DITERATIONS={int(iterations)}", "-I", b / "cm", "-I", mw / "include", "-w"]
    objs = []
    for f in ("core_list_join", "core_main", "core_matrix", "core_state", "core_util", "ee_printf", "core_portme", "console"):
        run(cc + flags + ["-c", b / "cm" / f"{f}.c", "-o", b / f"{f}.o"], b, f"compiling {f}.c")
        objs.append(b / f"{f}.o")
    run(cc + flags + ["-c", b / "cm" / "head.S", "-o", b / "head.o"], b, "assembling head.S")
    run(cc + ["-nostdlib", "-static", "-Wl,-T," + str(port / "powerpc.lds"), "-o", b / "coremark.elf", b / "head.o"] + objs, b, "linking the Microwatt CoreMark image")
    run(objcopy + ["-O", "binary", b / "coremark.elf", b / "main_ram.bin"], b, "objcopy")
    size = (b / "main_ram.bin").stat().st_size
    if size > 380 * 1024:
        raise Fail(f"microwatt_coremark: the image is {size} bytes, the testbench memory holds 384 KiB")
    hexdata = run([sys.executable, "-I", mw / "scripts" / "bin2hex.py", b / "main_ram.bin"], b, "bin2hex.py")
    (b / "main_ram.hex").write_text(hexdata)
    # the generated git.vhdl package (the upstream Makefile writes it from git.vhdl.in): a fixed hash, so images do not depend on a checkout state
    (b / "git.vhdl").write_text((mw / "git.vhdl.in").read_text().replace("@hash@", head("microwatt")[:14]).replace("@dirty@", "0"))
    (b / "build.txt").write_text(f"{version}\niterations {iterations}\nimage {size} bytes\n")
    for junk in ("cm", "coremark.elf", "main_ram.bin", *[o.name for o in objs], "head.o"):
        p = b / junk
        shutil.rmtree(p, ignore_errors=True) if p.is_dir() else p.unlink(missing_ok=True)
    shutil.rmtree(d, ignore_errors=True)
    b.rename(d)
    return d


PROGRAMS = {"neorv32_coremark": neorv32_coremark, "microwatt_coremark": microwatt_coremark}


def build(spec):
    """spec: {"program": name, ...parameters}. Returns the image directory."""
    name = spec.get("program")
    if name not in PROGRAMS:
        raise Fail(f"unknown program `{name}` (known: {', '.join(PROGRAMS)})")
    try:
        return PROGRAMS[name](**{k: v for k, v in spec.items() if k != "program"})
    except Fail:
        for tmp in (CACHE / "images").glob(f"{name}-*.tmp"):  # a failed build leaves its scratch directory behind
            shutil.rmtree(tmp, ignore_errors=True)
        raise


def main():
    if len(sys.argv) < 2:
        print(__doc__)
        return 2
    spec = {"program": sys.argv[1]}
    for kv in sys.argv[2:]:
        k, v = kv.split("=", 1)
        spec[k] = int(v) if re.fullmatch(r"\d+", v) else v
    try:
        print(build(spec))
    except Fail as e:
        print(f"programs: {e}", file=sys.stderr)
        return 2
    return 0


if __name__ == "__main__":
    sys.exit(main())

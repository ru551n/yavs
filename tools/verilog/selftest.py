#!/usr/bin/env python3
"""Self-test of the Verilog test tools against the stand-in analyzer fake_yavs.py (no vendor install, no real analyzer needed).

  python3 -I tools/verilog/selftest.py

Checks that fixtures.py and corpus.py pass on a correct stand-in, fail (exit 1, with a DIFF) on each deliberately wrong one,
and skip with a notice when the install is absent. Exit 0 when all of that holds.
"""
import os, subprocess, sys, tempfile
from pathlib import Path

HERE = Path(__file__).resolve().parent
FAKE = f"{sys.executable} -I {HERE / 'fake_yavs.py'}"
ENV = "`pragma protect begin_protected\n`pragma protect data_block\nQUJD \"unterminated /*\n`pragma protect end_protected\n"
HDR = "`timescale 1ns/1ps\n"
TREE = {
    "verilog/src/glbl.v": HDR + "module glbl; wire GSR; assign GSR = 1'b0; endmodule\n",
    "verilog/src/unisims/A.v": HDR + "module A (input i, output o); assign o = i; endmodule\n",
    "verilog/src/unisims/B.v": HDR + "module B (input i, output o); assign o = ~i; endmodule\n",
    "verilog/src/unisims/S.sv": "module S; endmodule\n",
    "verilog/src/retarget/R.v": HDR + "module R (input i, output o); assign o = i; endmodule\n",
    "verilog/src/unimacro/M.v": HDR + "module M (input i, output o); assign o = i; endmodule\n",
    "verilog/src/unifast/A.v": HDR + "module A (input i, output o); assign o = i; endmodule\n",
    "verilog/src/unisim_retarget_comp.vp": HDR + "module P; endmodule\n" + ENV,
    "secureip/x/E.vp": HDR + "module E; endmodule\n" + ENV,
    "ip/xilinx/foo/hdl/plain.v": HDR + '`include "defs.vh"\nmodule plain (input i, output o); assign o = `INV(i); endmodule\n',
    "ip/xilinx/foo/hdl/enc.v": ENV,
    "ip/xilinx/foo/hdl/svfile.v": HDR + "module svfile (input clk, d, output reg q); always_ff @(posedge clk) q <= d; endmodule\n",
    "ip/xilinx/foo/hdl/useenc.v": HDR + '`include "enc_defs.v"\nmodule useenc; assign x = 1; endmodule\n',
    "ip/xilinx/foo/hdl/enc_defs.v": ENV,
    "ip/xilinx/bar/hdl/defs.vh": "`define INV(x) (~(x))\n",
}


def run(cmd, env=None, expect=0, want_text=()):
    p = subprocess.run(cmd, capture_output=True, text=True, env={**os.environ, **(env or {})})
    ok = p.returncode == expect and all(w in p.stdout for w in want_text)
    print(("ok   " if ok else "FAIL ") + " ".join(cmd[2:])[:150] + (f"  [{env}]" if env else ""))
    if not ok:
        print(f"     exit {p.returncode}, wanted {expect}; wanted text {want_text}\n" + p.stdout[-1500:] + p.stderr[-500:])
    return ok


def main():
    py = [sys.executable, "-I"]
    fx, co = py + [str(HERE / "fixtures.py")], py + [str(HERE / "corpus.py")]
    ok = run(fx + ["--static"])
    ok &= run(fx + ["--yavs", FAKE, "--no-wrap"], want_text=["29 fixtures, 0 differences"])
    bugs = ("double-e4001", "miss-e4001", "plain-error")
    with tempfile.TemporaryDirectory(prefix="yavs-verilog-selftest-") as tmp:
        root = Path(tmp) / "data"
        for rel, text in TREE.items():
            (root / rel).parent.mkdir(parents=True, exist_ok=True)
            (root / rel).write_text(text)
        man = Path(tmp) / "vendor.expected"
        base = co + ["vendor", "--root", str(root), "--manifest", str(man)]
        ok &= run(base + ["gen"], want_text=["ip-includes-encrypted", "protected"])
        txt = man.read_text()
        for want in ("E4001 ip ", "E4002 work", "E4003 ip", "PASS unisims_ver timing"):
            if want not in txt:
                print("FAIL manifest lacks", want)
                ok = False
        chk = base + ["--yavs", FAKE, "--no-wrap"]
        ok &= run(chk, want_text=["outcomes:"])
        for b in bugs:
            ok &= run(chk, env={"FAKE_YAVS_BUG": b}, expect=1, want_text=["DIFF"])
            ok &= run(fx + ["--yavs", FAKE, "--no-wrap"], env={"FAKE_YAVS_BUG": b}, expect=1, want_text=["FAIL"])
        ok &= run(co + ["vendor", "--root", str(Path(tmp) / "none")], env={"HOME": tmp, "YAVS_VENDOR_DATA": ""}, want_text=["skipped"])
        ok &= run(co + ["cells", "--root", str(Path(tmp) / "none")], env={"HOME": tmp, "YAVS_CORPUS": ""}, want_text=["skipped"])
    print("selftest", "passed" if ok else "FAILED")
    return 0 if ok else 1


if __name__ == "__main__":
    sys.exit(main())

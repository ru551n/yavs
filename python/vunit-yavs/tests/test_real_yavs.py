# This Source Code Form is subject to the terms of the Mozilla Public
# License, v. 2.0. If a copy of the MPL was not distributed with this file,
# You can obtain one at http://mozilla.org/MPL/2.0/.
#
# Copyright (c) 2014-2026, Lars Asplund lars.anders.asplund@gmail.com

"""
End to end on a real yavs (VUNIT_YAVS_PATH, else yavs on PATH): the project's run.py, plain and under
--yavs-debug with a minimal DAP client.
"""

import json
import os
import queue
import shutil
import socket
import subprocess
import sys
import tempfile
import threading
import time
import unittest
from pathlib import Path

from test_fake_e2e import COMMA, FAIL, PASS, report
from vunit.vunit_cli import VUnitCLI
from vunit_yavs.console import HELP
from vunit_yavs.tokens import TokenServer

HERE = Path(__file__).parent
YAVS_DIR = os.environ.get("VUNIT_YAVS_PATH") or (Path(shutil.which("yavs")).parent if shutil.which("yavs") else None)
TIMEOUT = 280

# 64 registers on one clock: with YAVS_POOL_ALL=1 every rising edge goes to the threads
TOK_DESIGN = """
entity tok is end;
architecture a of tok is
  signal clk : bit := '0';
  type arr is array (0 to 63) of natural;
  signal r : arr := (others => 0);
begin
  clk <= not clk after 5 ns when now < 3 us;
  g : for i in 0 to 63 generate
    process (clk) begin
      if clk'event and clk = '1' then
        r(i) <= (r(i) * 7 + i + 1) mod 65521;
      end if;
    end process;
  end generate;
  process begin
    wait for 3 us + 1 ns;
    report "r(63) " & integer'image(r(63));
    wait;
  end process;
end;
"""


NON_ASCII_PACKAGE = """
package pkg is
  constant answer : integer := 42;
end package;
"""

NON_ASCII_TESTBENCH = """
library vunit_lib;
context vunit_lib.vunit_context;
use work.pkg.all;
entity tb is
  generic (runner_cfg : string);
end entity;
architecture a of tb is
begin
  main : process
  begin
    test_runner_setup(runner, runner_cfg);
    check_equal(answer, 42);
    test_runner_cleanup(runner);
    wait;
  end process;
end architecture;
"""

NON_ASCII_RUN = """
from pathlib import Path
from vunit import VUnit
VU = VUnit.from_argv()
VU.add_package("vunit-yavs", allow_setup=True)
VU.add_vhdl_builtins()
VU.add_library("lib").add_source_files(Path(__file__).parent / "src-é" / "*.vhd")
VU.main()
"""


def send(sock, message):
    """Send one DAP message"""
    body = json.dumps(message).encode()
    sock.sendall(f"Content-Length: {len(body)}\r\n\r\n".encode() + body)


def receive(stream):
    """Read one DAP message from a socket file, or None at the end of the stream"""
    length = None
    while True:
        line = stream.readline()
        if not line:
            return None
        line = line.strip()
        if not line:
            break
        name, _, value = line.decode().partition(":")
        if name.lower() == "content-length":
            length = int(value)
    return json.loads(stream.read(length))


@unittest.skipIf(YAVS_DIR is None, "no yavs: set VUNIT_YAVS_PATH or put yavs on PATH")
class TestRealYavs(unittest.TestCase):
    """
    Run run.py with VUNIT_SIMULATOR=yavs on the real simulator
    """

    def setUp(self):
        self._tempdir = tempfile.TemporaryDirectory()  # pylint: disable=consider-using-with
        self.addCleanup(self._tempdir.cleanup)
        self.tempdir = Path(self._tempdir.name)
        self.xml = self.tempdir / "x.xml"

    def _command(self, *args):
        return [sys.executable, str(HERE / "project" / "run.py"), "-o", str(self.tempdir / "out")] + [
            "-x",
            str(self.xml),
            *args,
        ]

    @staticmethod
    def _env():
        return dict(
            os.environ,
            VUNIT_SIMULATOR="yavs",
            VUNIT_YAVS_PATH=str(YAVS_DIR),
        )

    def test_guide_example(self):
        """docs/examples/vunit, the run script of the user guide: plain and with coverage"""
        example = HERE.parents[2] / "docs" / "examples" / "vunit" / "run.py"
        if not example.exists():
            self.skipTest("Not run from the yavs repository")
        coverage = ["--yavs-cov", "--yavs-cov-html", "cov-html", "--yavs-cov-fail-under", "85"]
        for flags in ([], coverage):
            proc = subprocess.run(
                [sys.executable, str(example), "-p", "2", "-x", str(self.xml), *flags],
                cwd=self.tempdir,
                env=self._env(),
                stdout=subprocess.PIPE,
                stderr=subprocess.STDOUT,
                text=True,
                timeout=TIMEOUT,
                check=False,
            )
            self.assertEqual(proc.returncode, 0, proc.stdout)
            self.assertEqual(list(report(self.xml).values()), ["passed"] * 4, proc.stdout)
        self.assertTrue((self.tempdir / "coverage.ycov").exists())
        self.assertTrue((self.tempdir / "cov-html" / "index.html").exists())

    def test_run(self):
        proc = subprocess.run(
            self._command("--yavs-max-threads=2"),
            env=self._env(),
            stdout=subprocess.PIPE,
            stderr=subprocess.STDOUT,
            text=True,
            timeout=TIMEOUT,
            check=False,
        )
        self.assertEqual(proc.returncode, 1, proc.stdout)
        self.assertEqual(report(self.xml), {PASS: "passed", COMMA: "passed", FAIL: "failed"}, proc.stdout)

    def test_lint_after_compile(self):
        def run(*args):
            return subprocess.run(
                self._command("--compile", *args),
                env=self._env(),
                stdout=subprocess.PIPE,
                stderr=subprocess.STDOUT,
                encoding="utf-8",
                errors="replace",
                timeout=TIMEOUT,
                check=False,
            )

        proc = run("--yavs-lint")
        self.assertEqual(proc.returncode, 0, proc.stdout)
        self.assertIn("finding", proc.stdout)
        # A library with a finding fails the compile step, and is not checked without the flag.
        design = self.tempdir / "bad.vhd"
        design.write_text("entity bad is end;\narchitecture a of bad is\nbegin\n  process begin report \"x\"; end process;\nend;\n")
        env = dict(self._env(), EXTRA_VHDL=str(design))
        self.assertEqual(
            subprocess.run(self._command("--compile"), env=env, stdout=subprocess.DEVNULL, timeout=TIMEOUT, check=False).returncode,
            0,
        )
        proc = subprocess.run(
            self._command("--compile", "--yavs-lint"),
            env=env,
            stdout=subprocess.PIPE,
            stderr=subprocess.STDOUT,
            encoding="utf-8",
            errors="replace",
            timeout=TIMEOUT,
            check=False,
        )
        self.assertEqual(proc.returncode, 1, proc.stdout)
        self.assertIn("lint_770", proc.stdout)
        # The configuration of the directory it runs in applies, and an excluded library is no error.
        (self.tempdir / "yavs-lint.yaml").write_text("exclude: ['**/bad.vhd', '**/vunit/vhdl/**']\n")
        proc = subprocess.run(
            self._command("--compile", "--yavs-lint"),
            env=env,
            cwd=self.tempdir,
            stdout=subprocess.PIPE,
            stderr=subprocess.STDOUT,
            encoding="utf-8",
            errors="replace",
            timeout=TIMEOUT,
            check=False,
        )
        self.assertEqual(proc.returncode, 0, proc.stdout)
        self.assertNotIn("lint_770", proc.stdout)

    def _coverage_run(self, *args, **env):
        return subprocess.run(
            self._command("--yavs-cov", PASS, COMMA, *args),
            env=dict(self._env(), RUN_PY_MERGE_COVERAGE=str(self.tempdir / "all"), **env),
            stdout=subprocess.PIPE,
            stderr=subprocess.STDOUT,
            text=True,
            timeout=TIMEOUT,
            check=False,
        )

    def test_coverage_merged_and_reported(self):
        proc = self._coverage_run("--yavs-cov-html", str(self.tempdir / "html"), "--yavs-cov-fail-under=0")
        self.assertEqual(proc.returncode, 0, proc.stdout)
        self.assertEqual(report(self.xml), {PASS: "passed", COMMA: "passed"})
        self.assertTrue((self.tempdir / "all.ycov").is_file(), proc.stdout)
        self.assertTrue((self.tempdir / "html" / "index.html").is_file(), proc.stdout)
        # The merged database names both tests and the report is the usual one
        database = (self.tempdir / "all.ycov").read_text(encoding="utf-8")
        self.assertIn(f"\t{PASS}\n", database)
        self.assertIn(f"\t{COMMA}\n", database)
        self.assertIn("statements ", proc.stdout)

    def test_coverage_below_the_threshold_fails_after_the_reports(self):
        proc = self._coverage_run("--yavs-cov-fail-under=100")
        self.assertEqual(proc.returncode, 1, proc.stdout)
        # VUnit still wrote its report, and yavs said why the run failed
        self.assertEqual(report(self.xml), {PASS: "passed", COMMA: "passed"})
        self.assertRegex(proc.stdout, r"error\[E0114\]: coverage [0-9.]+% is below the required 100.0%")

    def test_non_ascii_file_names_compile_in_batches(self):
        # Several files in one library with non-ASCII names go in one call, also with the runner's own
        # --batch-compile and --parallel-compile where it has them (#114)
        src = self.tempdir / "src-é"
        src.mkdir()
        (src / "pkg_ü.vhd").write_text(NON_ASCII_PACKAGE, encoding="utf-8")
        (src / "tb_ñ.vhd").write_text(NON_ASCII_TESTBENCH, encoding="utf-8")
        run = self.tempdir / "run.py"
        run.write_text(NON_ASCII_RUN, encoding="utf-8")
        flags = [flag for flag in ("--batch-compile", "--parallel-compile") if flag in VUnitCLI().parser.format_help()]
        for extra in ([], flags):
            with self.subTest(flags=extra):
                proc = subprocess.run(
                    [sys.executable, str(run), "-o", str(self.tempdir / "out"), "-x", str(self.xml), *extra],
                    env=self._env(),
                    stdout=subprocess.PIPE,
                    stderr=subprocess.STDOUT,
                    text=True,
                    timeout=TIMEOUT,
                    check=False,
                )
                self.assertEqual(proc.returncode, 0, proc.stdout)
                self.assertEqual(list(report(self.xml).values()), ["passed"], proc.stdout)
                self.assertNotIn("in one call failed", proc.stdout)
                shutil.rmtree(self.tempdir / "out")

    def test_response_file_in_the_locale_encoding_is_a_clear_error(self):
        # What a runner writing a file list in a Windows code page produces: yavs says what is wrong
        (self.tempdir / "list.txt").write_bytes("é.vhd\n".encode("latin-1"))
        proc = subprocess.run(
            [str(Path(YAVS_DIR) / "yavs"), "analyze", "--std=2008", f"--work=work:{self.tempdir / 'lib'}"]
            + ["@" + str(self.tempdir / "list.txt")],
            capture_output=True,
            text=True,
            timeout=TIMEOUT,
            check=False,
        )
        self.assertEqual(proc.returncode, 2, proc.stderr)
        self.assertIn("error[E0101]", proc.stderr)
        self.assertIn("UTF-8", proc.stderr)

    def test_run_with_token_server(self):
        # Each simulation runs with --threads=2 and connects; a bad YAVS_TOKENS would be error E0128
        proc = subprocess.run(
            self._command("-p", "2", "--yavs-jobserver", "-v", PASS, COMMA),
            env=self._env(),
            stdout=subprocess.PIPE,
            stderr=subprocess.STDOUT,
            text=True,
            timeout=TIMEOUT,
            check=False,
        )
        self.assertEqual(proc.returncode, 0, proc.stdout)
        self.assertEqual(report(self.xml), {PASS: "passed", COMMA: "passed"})
        self.assertIn("--threads=2", proc.stdout)
        self.assertNotIn("E0128", proc.stdout)

    def test_yavs_runs_on_lent_tokens(self):
        # A design whose every clock edge is worth the threads: yavs takes the token, output unchanged
        (self.tempdir / "tok.vhd").write_text(TOK_DESIGN)
        lib = f"--work=work:{self.tempdir / 'lib'}"
        yavs = str(Path(YAVS_DIR) / "yavs")
        std_cache = f"--std-cache={self.tempdir / 'std'}"
        subprocess.run([yavs, "analyze", "--std=2008", std_cache, lib, str(self.tempdir / "tok.vhd")], check=True)
        server = TokenServer(2)
        self.addCleanup(server.close)

        def run(threads, env):
            return subprocess.run(
                [yavs, "run", std_cache, lib, f"--threads={threads}", "tok"],
                env=dict(env, YAVS_POOL_ALL="1", YAVS_COUNTERS="1"),
                capture_output=True,
                text=True,
                timeout=TIMEOUT,
                check=True,
            )

        def pooled(proc):
            lines = [line for line in proc.stderr.splitlines() if line.startswith("counters: pooled_batches=")]
            self.assertEqual(len(lines), 1, proc.stderr)
            return int(lines[0].split("=")[1])

        alone = run(1, os.environ)
        client = server.add()
        lent = run(2, server.env(client, os.environ))
        server.remove(client)
        self.assertEqual(lent.stdout, alone.stdout)
        self.assertGreater(pooled(lent), 0)
        # Two tests running on -p 2: no free slot, so nothing goes to the threads
        busy = server.add()
        client = server.add()
        none = run(2, server.env(client, os.environ))
        server.remove(client)
        server.remove(busy)
        self.assertEqual(none.stdout, alone.stdout)
        self.assertEqual(pooled(none), 0)

    def _debug_session(self):
        """
        Run one test under --yavs-debug=0, attach, initialize, launch, run to the end and disconnect.
        Return the DAP events, the run's exit code, and VUnit's exit code and output.
        """
        proc = subprocess.Popen(  # pylint: disable=consider-using-with
            self._command("--yavs-debug=0", PASS),
            env=self._env(),
            stdout=subprocess.PIPE,
            stderr=subprocess.STDOUT,
            text=True,
        )
        self.addCleanup(proc.kill)  # A failed check must not leave the run waiting for a debugger
        lines = queue.Queue()
        output = []
        threading.Thread(target=lambda: [lines.put(line) for line in proc.stdout] + [lines.put(None)]).start()

        # The package shows where to attach; port 0 makes yavs pick a free port
        deadline = time.monotonic() + TIMEOUT
        address = None
        while address is None:
            line = lines.get(timeout=max(1, deadline - time.monotonic()))
            self.assertIsNotNone(line, "".join(output))
            output.append(line)
            if f"Debugging {PASS}: attach a DAP client to " in line:
                address = line.rsplit(" ", 1)[1].strip()
        host, port = address.rsplit(":", 1)

        events, exit_code = [], None
        with socket.create_connection((host, int(port)), timeout=60) as sock:
            stream = sock.makefile("rb")
            send(sock, {"seq": 1, "type": "request", "command": "initialize", "arguments": {"adapterID": "yavs"}})
            response = receive(stream)
            self.assertEqual((response["command"], response["success"]), ("initialize", True))
            self.assertTrue(response["body"]["supportsConfigurationDoneRequest"])
            send(sock, {"seq": 2, "type": "request", "command": "launch", "arguments": {}})
            send(sock, {"seq": 3, "type": "request", "command": "configurationDone"})
            while not events or events[-1]["event"] != "terminated":
                message = receive(stream)
                self.assertIsNotNone(message, events)
                if message["type"] == "event":
                    events.append(message)
                    if message["event"] == "exited":
                        exit_code = message["body"]["exitCode"]
            send(sock, {"seq": 4, "type": "request", "command": "disconnect"})

        output += list(iter(lines.get, None))
        return events, exit_code, proc.wait(timeout=TIMEOUT), "".join(output)

    def test_surfer_gets_the_wave_file_of_the_test(self):
        frames = []
        listener = socket.create_server(("127.0.0.1", 0))
        self.addCleanup(listener.close)

        def fake_surfer():
            conn, _ = listener.accept()
            with conn, conn.makefile("rb") as stream:
                buffer = b""
                while True:
                    byte = stream.read(1)
                    if not byte:
                        return
                    if byte != b"\0":
                        buffer += byte
                        continue
                    message = json.loads(buffer)
                    buffer = b""
                    frames.append(message)
                    if message["type"] == "greeting":
                        answer = {"type": "greeting", "version": "0", "commands": []}
                    elif message["command"] == "add_items":
                        answer = {"type": "response", "command": "add_items", "ids": list(range(len(message["items"])))}
                    else:
                        answer = {"type": "response", "command": "ack"}
                    conn.sendall(json.dumps(answer).encode() + b"\0")
                    if message.get("command") in ("load", "reload"):
                        event = {"type": "event", "event": "waveforms_loaded", "source": "x"}
                        conn.sendall(json.dumps(event).encode() + b"\0")

        thread = threading.Thread(target=fake_surfer)
        thread.start()
        port = listener.getsockname()[1]
        proc = subprocess.run(
            self._command(f"--yavs-surfer=127.0.0.1:{port}", PASS),
            env=self._env(),
            stdout=subprocess.PIPE,
            stderr=subprocess.STDOUT,
            text=True,
            timeout=TIMEOUT,
            check=False,
        )
        thread.join(timeout=TIMEOUT)
        self.assertEqual(proc.returncode, 0, proc.stdout)
        load = next(f for f in frames if f.get("command") == "load")
        self.assertTrue(load["source"].endswith("wave.fst"), load)
        self.assertTrue(Path(load["source"]).exists())
        self.assertEqual(frames[-1]["command"], "zoom_to_fit")

    def test_surfer_refuses_parallel_runs(self):
        proc = subprocess.run(
            self._command("--yavs-surfer", "-p", "2", PASS),
            env=self._env(),
            stdout=subprocess.PIPE,
            stderr=subprocess.STDOUT,
            text=True,
            timeout=TIMEOUT,
            check=False,
        )
        self.assertNotEqual(proc.returncode, 0, proc.stdout)
        self.assertIn("--yavs-surfer shows one test", proc.stdout)

    def _preprocessed_project(self):
        """A project with location and check preprocessing on, and the line of the marker statement"""
        project = self.tempdir / "pp"
        project.mkdir()
        project = project.resolve()  # A short temporary path on Windows is not what a client sends
        text = (
            "library vunit_lib;\ncontext vunit_lib.vunit_context;\n\n"
            "entity tb_pp is\n  generic (runner_cfg : string);\nend entity;\n\n"
            "architecture tb of tb_pp is\nbegin\n  main : process\n    variable x : integer := 3;\n  begin\n"
            "    test_runner_setup(runner, runner_cfg);\n"
            '    info("before");\n'
            "    check(x = 3,\n"
            '      "several lines");\n'
            '    check_equal(x, 3, "another check");\n'
            "    x := 4;  -- marker\n"
            "    check(x = 4);\n"
            "    test_runner_cleanup(runner);\n  end process;\nend architecture;\n"
        )
        source = project / "tb_pp.vhd"
        source.write_text(text)
        (project / "run.py").write_text(
            "from pathlib import Path\nfrom vunit import VUnit\n"
            "VU = VUnit.from_argv()\n"
            'VU.add_package("vunit-yavs", allow_setup=True)\n'
            "VU.add_vhdl_builtins()\n"
            "VU.enable_location_preprocessing()\n"
            "VU.enable_check_preprocessing()\n"
            'VU.add_library("lib").add_source_files(Path(__file__).parent / "tb_pp.vhd")\n'
            "VU.main()\n"
        )
        line = text.splitlines().index("    x := 4;  -- marker") + 1
        return project, source, line

    def test_preprocessed_file_at_the_console(self):
        project, source, line = self._preprocessed_project()
        code, out = self._session(project, project / "run.py", "lib.tb_pp.all", f"break tb_pp.vhd:{line}\nrun\nexamine x\nwhere\nrun\nquit\n")
        self.assertIn(f"breakpoint 1: {source}:{line}\n", out)
        self.assertIn(f"{source}:{line}\n3\n", out, "stopped before the marker, in the original file")
        preprocessed = Path(self.tempdir / "out" / "preprocessed" / "lib" / "tb_pp.vhd")
        self.assertTrue(preprocessed.exists())
        self.assertNotEqual(preprocessed.read_text(), source.read_text(), "the copy is what is compiled")
        self.assertEqual(
            preprocessed.read_text().splitlines()[line - 1].strip(), "x := 4;  -- marker", "the copy keeps the lines"
        )
        self.assertNotIn(str(preprocessed), out.split("breakpoint 1")[1].split("Starting")[0])
        self.assertEqual(code, 0, out)

    def test_preprocessed_file_in_a_debug_client(self):
        project, source, line = self._preprocessed_project()
        proc = subprocess.Popen(  # pylint: disable=consider-using-with
            [
                sys.executable,
                str(project / "run.py"),
                "-o",
                str(self.tempdir / "out"),
                "--yavs-debug=0",
                "lib.tb_pp.all",
            ],
            cwd=project,
            env=self._env(),
            stdout=subprocess.PIPE,
            stderr=subprocess.STDOUT,
            text=True,
        )
        self.addCleanup(proc.wait)
        self.addCleanup(proc.kill)
        address = None
        output = []
        for out_line in proc.stdout:
            output.append(out_line)
            if ": attach a DAP client to " in out_line:
                address = out_line.rsplit(" ", 1)[1].strip()
                break
        self.assertIsNotNone(address, "".join(output))
        host, port = address.rsplit(":", 1)
        # As the extension looks first: a closed connection is no session
        socket.create_connection((host, int(port)), timeout=60).close()
        with socket.create_connection((host, int(port)), timeout=60) as sock:
            stream = sock.makefile("rb")

            def until(check):
                while True:
                    message = receive(stream)
                    self.assertIsNotNone(message, "".join(output))
                    if check(message):
                        return message

            def request(seq, command, **arguments):
                send(sock, {"seq": seq, "type": "request", "command": command, "arguments": arguments})
                return until(lambda m: m["type"] == "response" and m["request_seq"] == seq)

            request(1, "initialize", adapterID="yavs")
            request(2, "attach")
            answer = request(3, "setBreakpoints", source={"path": str(source)}, breakpoints=[{"line": line}])
            self.assertTrue(answer["body"]["breakpoints"][0]["verified"], answer)
            request(4, "configurationDone")
            stopped = until(lambda m: m.get("event") == "stopped")
            self.assertEqual(stopped["body"]["reason"], "breakpoint")
            frames = request(5, "stackTrace", threadId=stopped["body"]["threadId"])["body"]["stackFrames"]
            self.assertEqual((frames[0]["source"]["path"], frames[0]["line"]), (str(source), line))
            # A vunit_* command in the Debug Console
            request(6, "evaluate", expression="vunit_user_init", context="repl")
            until(lambda m: m.get("event") == "output" and m["body"]["output"] == "0\n")
            request(7, "setBreakpoints", source={"path": str(source)}, breakpoints=[])
            request(8, "continue", threadId=stopped["body"]["threadId"])
            until(lambda m: m.get("event") == "terminated")
            request(9, "disconnect")
        proc.stdout.read()
        self.assertEqual(proc.wait(timeout=TIMEOUT), 0)

    def _session(self, cwd, run_py, test, stdin, *args):
        """A --yavs-interactive session with these console lines: its exit status and output"""
        proc = subprocess.run(
            [sys.executable, str(run_py), "--no-color", "-o", str(self.tempdir / "out"), "-x", str(self.xml)]
            + list(args)
            + ["--yavs-interactive", test],
            env=self._env(),
            cwd=cwd,
            input=stdin,
            stdout=subprocess.PIPE,
            stderr=subprocess.STDOUT,
            text=True,
            timeout=TIMEOUT,
            check=False,
        )
        return proc.returncode, proc.stdout

    def _copy_project(self):
        project = self.tempdir / "project"
        project.mkdir()
        shutil.copy(HERE / "project" / "run.py", project)
        shutil.copy(HERE / "project" / "tb_yavs.vhd", project)
        return project

    def test_interactive_commands(self):
        project = self._copy_project()
        code, out = self._session(project, project / "run.py", PASS, "vunit_run\nquit\n")
        self.assertIn(HELP, out, "the help after the load")
        self.assertIn("false", out.split("STOP called")[1], out)
        self.assertEqual(code, 0, out)
        self.assertEqual(report(self.xml), {PASS: "passed"}, out)
        # The console reaches the result of a batch run of the same test
        batch = subprocess.run(
            self._command(FAIL), env=self._env(), capture_output=True, text=True, timeout=TIMEOUT, check=False
        )
        code, out = self._session(project, project / "run.py", FAIL, "vunit_run\nquit\n")
        self.assertEqual(code, batch.returncode, out)
        self.assertEqual(report(self.xml), {FAIL: "failed"}, out)
        self.assertIn("Test Run Failed!", out)
        self.assertIn("\nTest Run Failed!\n\ntrue\n", out)

    def test_interactive_refuses(self):
        for args in [("-p", "2"), ("--yavs-debug=0",), ("--elaborate",)]:
            code, out = self._session(HERE / "project", HERE / "project" / "run.py", PASS, "", *args)
            self.assertEqual(code, 1, out)
            self.assertIn("--yavs-interactive", out, args)
        code, out = self._session(HERE / "project", HERE / "project" / "run.py", "lib.tb_yavs.*", "")
        self.assertNotEqual(code, 0, out)
        self.assertIn("runs one simulation", out)

    def _with_edit(self, project, old, new):
        """A run script with a console command `edit` standing in for the user editing the source"""
        source = project / "tb_yavs.vhd"
        run_py = project / "run_edit.py"
        run_py.write_text(
            (HERE / "project" / "run.py")
            .read_text()
            .replace("__file__", repr(str(project / "run.py")))
            .replace(
                "VU.main(",
                "from pathlib import Path\n"
                "import vunit_yavs.console as c\n"
                "_init = c.VunitCommands.__init__\n"
                "def edit(_):\n"
                f"    p = Path({str(source)!r})\n"
                f"    p.write_text(p.read_text().replace({old!r}, {new!r}, 1))\n"
                "    return ''\n"
                "def init(self, *a, **k):\n"
                "    _init(self, *a, **k)\n"
                "    self.hook._commands['edit'] = ('edit the source', edit)\n"
                "c.VunitCommands.__init__ = init\n"
                "VU.main(",
            )
        )
        return run_py

    def test_interactive_restart_compiles_and_uses_the_new_code(self):
        project = self._copy_project()
        run_py = self._with_edit(project, "check_true(true);", 'report "new code";')
        lines = "break tb_yavs.vhd:19\nvunit_run\nbreak delete all\nrun\nedit\nvunit_restart\nquit\n"
        code, out = self._session(project, run_py, PASS, lines)
        self.assertIn("\nTest Run Failed!\n\ntrue\n", out, "a breakpoint stops the run before the suite ends")
        self.assertIn(f"Re-compiling using command {sys.executable} -u {run_py} --compile --no-color", out)
        self.assertIn("Re-compile finished\n", out)
        self.assertIn("new code", out, "the restart ran the edited source")
        self.assertEqual(code, 0, out)
        self.assertEqual(report(self.xml), {PASS: "passed"}, out)

    def test_interactive_compile_failure_keeps_the_design(self):
        project = self._copy_project()
        run_py = self._with_edit(project, "check_true(true);", "this does not parse")
        lines = "run 0 ns\nedit\nvunit_compile\nvunit_restart\nwhere\nvunit_run\nquit\n"
        code, out = self._session(project, run_py, PASS, lines)
        self.assertIn("Re-compile failed\nchild process exited abnormally\ntrue\n", out)
        # vunit_restart stops at the failed compile, and the old design runs to the end
        self.assertEqual(out.count("Re-compile failed"), 2, out)
        self.assertIn("STOP called", out)
        self.assertEqual(code, 0, out)

    def test_interactive_init_files(self):
        project = self._copy_project().resolve()
        (project / "after.do").write_text("# $vunit_tb_name\nexamine ${vunit_tb_name}_unknown_is_an_error\n")
        (project / "before.do").write_text("where\n")
        (project / "gui.do").write_text("where\n")
        run_py = project / "run_init.py"
        run_py.write_text(
            (HERE / "project" / "run.py")
            .read_text()
            .replace("__file__", repr(str(project / "run.py")))
            .replace(
                "VU.main(",
                "lib = VU.library('lib')\n"
                f"lib.set_sim_option('yavs.init_files.after_load', [{str(project / 'after.do')!r}])\n"
                f"lib.set_sim_option('yavs.init_files.before_run', [{str(project / 'before.do')!r}])\n"
                f"lib.set_sim_option('yavs.init_file.gui', {str(project / 'gui.do')!r})\n"
                "VU.main(",
            )
        )
        code, out = self._session(project, run_py, PASS, "vunit_user_init\nvunit_run\nquit\n")
        posix = project.as_posix()
        self.assertIn(f"Sourcing file {posix}/after.do from yavs.init_files.after_load\n", out)
        self.assertIn(f"Sourcing {posix}/after.do failed\n", out)
        self.assertIn("tb_yavs_unknown_is_an_error", out)
        # A failed load: no GUI init and no help, as in VUnit's GUI script
        self.assertNotIn("List of VUnit commands", out)
        self.assertIn(f"Sourcing file {posix}/gui.do from yavs.init_file.gui\n", out)
        self.assertIn(f"Sourcing file {posix}/before.do from yavs.init_files.before_run\n", out)
        self.assertEqual(code, 0, out)

    def test_debug_handshake(self):
        events, exit_code, _, output = self._debug_session()
        self.assertEqual(events[0]["event"], "initialized")
        self.assertIsNotNone(exit_code, events)
        self.assertEqual(list(report(self.xml)), [PASS], output)
        log = (self.tempdir / "out").rglob("output.txt")
        self.assertTrue(any("yavs: DAP listening on" in path.read_text() for path in log))

    def test_debug_session_passes(self):
        events, exit_code, vunit_exit_code, output = self._debug_session()
        self.assertEqual(exit_code, 0, [event for event in events if event["event"] == "output"])
        self.assertEqual(vunit_exit_code, 0, output)
        self.assertEqual(report(self.xml), {PASS: "passed"})

    @unittest.skipUnless(
        Path(os.environ.get("XILINX_VIVADO", "/nowhere"), "data/vhdl/src/unisims").is_dir(),
        "no vendor install: set XILINX_VIVADO",
    )
    def test_vendor_libraries_of_an_install(self):
        command = [sys.executable, str(HERE / "project" / "run_vendor.py"), "-o", str(self.tempdir / "out")]
        command += ["-x", str(self.xml)]
        for _ in range(2):  # The second run has nothing to analyze
            proc = subprocess.run(
                command,
                env=self._env(),
                stdout=subprocess.PIPE,
                stderr=subprocess.STDOUT,
                text=True,
                timeout=TIMEOUT,
                check=False,
            )
            self.assertEqual(proc.returncode, 0, proc.stdout)
            self.assertEqual(report(self.xml), {"lib.tb_vendor.all": "passed"}, proc.stdout)

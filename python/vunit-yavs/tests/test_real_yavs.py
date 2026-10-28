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

HERE = Path(__file__).parent
YAVS_DIR = os.environ.get("VUNIT_YAVS_PATH") or (Path(shutil.which("yavs")).parent if shutil.which("yavs") else None)
TIMEOUT = 280


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

    def test_run_with_jobserver(self):
        # A yavs that reads no tokens leaves them in its pipe; the run ends with every token back
        proc = subprocess.run(
            self._command("-p", "2", "--yavs-jobserver", "--yavs-max-threads=2", PASS, COMMA),
            env=self._env(),
            stdout=subprocess.PIPE,
            stderr=subprocess.STDOUT,
            text=True,
            timeout=TIMEOUT,
            check=False,
        )
        self.assertEqual(proc.returncode, 0, proc.stdout)
        self.assertEqual(report(self.xml), {PASS: "passed", COMMA: "passed"})
        self.assertNotIn("jobserver", proc.stdout.replace("--yavs-jobserver", ""))

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

    def test_debug_handshake(self):
        events, exit_code, _, output = self._debug_session()
        self.assertEqual(events[0]["event"], "initialized")
        self.assertIsNotNone(exit_code, events)
        self.assertEqual(list(report(self.xml)), [PASS], output)
        log = (self.tempdir / "out").rglob("output.txt")
        self.assertTrue(any("yavs: DAP listening on" in path.read_text() for path in log))

    @unittest.expectedFailure  # yavs bug: under --dap the runner's active_python_runner is an internal error (E9999)
    def test_debug_session_passes(self):
        events, exit_code, vunit_exit_code, output = self._debug_session()
        self.assertEqual(exit_code, 0, [event for event in events if event["event"] == "output"])
        self.assertEqual(vunit_exit_code, 0, output)
        self.assertEqual(report(self.xml), {PASS: "passed"})

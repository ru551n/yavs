# This Source Code Form is subject to the terms of the Mozilla Public
# License, v. 2.0. If a copy of the MPL was not distributed with this file,
# You can obtain one at http://mozilla.org/MPL/2.0/.
#
# Copyright (c) 2014-2026, Lars Asplund lars.anders.asplund@gmail.com

"""
The VUnit commands of the yavs console: vunit_help, vunit_load, vunit_user_init, vunit_run,
vunit_compile and vunit_restart, with the wording and return values of VUnit's other simulator
interfaces, served through the console hook of yavs (YAVS_CONSOLE_HOOK, see the CLI contract).
"""

import json
import os
import re
import secrets
import shlex
import socket
import subprocess
import sys
import threading
from pathlib import Path

from vunit.test.suites import get_result_file_name

HOOK_VAR = "YAVS_CONSOLE_HOOK"
START = "_vunit_start"

HELP = """List of VUnit commands:
vunit_help
  - Prints this help
vunit_load [yavs_extra_args]
  - Loads design with correct generics for the test(s)
  - Optional first argument are passed as extra flags to yavs
  - Re-runs the user-defined after_load init file
vunit_user_init
  - Re-runs the user-defined GUI init file
vunit_run
  - Runs the user-defined before_load init file
  - Runs the test(s)
vunit_compile
  - Re-compiles the source files
vunit_restart
  - Re-compiles the source files
  - and re-runs the simulation if the compile was successful"""


class Hook:
    """
    The server side of one console session: yavs connects, gets the command list, and calls the commands.
    """

    def __init__(self, commands):
        self._commands = commands
        self._secret = secrets.token_hex(16)
        self._listener = socket.create_server(("127.0.0.1", 0))
        self._conn = None
        self._reader = None
        self._next = 0
        threading.Thread(target=self._serve, daemon=True).start()

    def env(self, env):
        """The environment of the yavs process"""
        port = self._listener.getsockname()[1]
        return dict(os.environ if env is None else env, **{HOOK_VAR: f"{port}:{self._secret}"})

    def close(self):
        """Stop serving"""
        self._listener.close()

    def _send(self, message):
        self._conn.sendall((json.dumps(message) + "\n").encode())

    def _serve(self):
        """Serve the first connection that knows the secret"""
        try:
            while True:
                self._conn, _ = self._listener.accept()
                self._reader = self._conn.makefile("r", encoding="utf-8", newline="\n")
                hello = json.loads(self._reader.readline() or "{}")
                if secrets.compare_digest(str(hello.get("secret")), self._secret):
                    break
                self._conn.close()
            self._send({"commands": [{"name": n, "help": h} for n, (h, _) in self._commands.items()]})
            self._loop()
        except (OSError, ValueError):
            pass  # yavs has gone; its exit status tells the run

    def _loop(self):
        for line in self._reader:
            message = json.loads(line)
            if "call" not in message:
                continue
            _, handler = self._commands[message["name"]]
            try:
                self._send({"done": message["call"], "result": handler(message["args"])})
            except Exception as exc:  # pylint: disable=broad-except
                self._send({"done": message["call"], "result": "", "error": str(exc)})

    def print(self, text=""):
        """Print on the console (the terminal or the editor's debug console)"""
        self._send({"print": text + "\n"})

    def _ask(self, message):
        self._next += 1
        message[next(iter(message))] = self._next
        self._send(message)
        answer = json.loads(self._reader.readline())
        return answer["ok"], answer["text"], answer["error"]

    def exec(self, line):
        """Run a console command; returns once it is done (a run: at the next stop): ok, text, error"""
        return self._ask({"exec": 0, "line": line})

    def reload(self, args):
        """Elaborate again from the libraries, at time 0, breakpoints kept: ok, text, error"""
        return self._ask({"reload": 0, "args": args})


class VunitCommands:
    """
    The commands of one test.
    """

    def __init__(self, output_path, config, argv=None, cwd=None):
        self._results = get_result_file_name(output_path)
        self._config = config
        argv = sys.argv if argv is None else argv
        self._compile_cmd = [sys.executable, "-u", argv[0], "--compile"] + [a for a in argv[1:] if a != "--clean"]
        self._cwd = Path(os.getcwd() if cwd is None else cwd).resolve()
        self.hook = Hook(
            {
                # The start of a session; no help text, so `help` does not list it
                START: ("", self._start),
                "vunit_help": ("list the VUnit commands", self.vunit_help),
                "vunit_load": ("load the test at time 0, VUnit's way", self.vunit_load),
                "vunit_user_init": ("re-run the GUI init file", self.vunit_user_init),
                "vunit_run": ("run the test, VUnit's way", self.vunit_run),
                "vunit_compile": ("re-compile through VUnit", self.vunit_compile),
                "vunit_restart": ("re-compile, then run the test again", self.vunit_restart),
            }
        )
        self._loaded = False

    def _source(self, file_name, option):
        """Source a file of console commands; True if it failed. VUnit's messages and variables."""
        path = Path(file_name).resolve()
        self.hook.print(f"Sourcing file {path.as_posix()} from {option}")

        def fail(error):
            self.hook.print(f"Sourcing {path.as_posix()} failed")
            self.hook.print(error)
            return True

        try:
            text = path.read_text(encoding="utf-8")
        except OSError as exc:
            return fail(str(exc))
        values = {
            "vunit_tb_path": Path(self._config.tb_path).resolve().as_posix(),
            "vunit_tb_name": self._config.design_unit_name,
        }
        text = re.sub(r"\$\{?(vunit_tb_path|vunit_tb_name)\}?", lambda m: values[m.group(1)], text)
        for line in text.splitlines():
            ok, out, error = self.hook.exec(line)
            if out:
                self.hook.print(out)
            if not ok:
                return fail(error)
        return False

    def _source_all(self, option):
        for file_name in self._config.sim_options.get(option, []):
            if self._source(file_name, option):
                return True
        return False

    def _start(self, _args):
        """As VUnit's GUI script: load; if that worked, the GUI init file and the help"""
        if self.vunit_load("") == "false":
            self.vunit_user_init("")
            self.vunit_help("")
        return ""

    def vunit_help(self, _args):
        self.hook.print(HELP)
        return ""

    def vunit_load(self, args):
        # yavs loaded the design when it started; only a later load elaborates again
        if self._loaded or args.strip():
            ok, _, error = self.hook.reload(shlex.split(args))
            if not ok:
                self.hook.print(error)
                return "true"
        self._loaded = True
        return "true" if self._source_all("yavs.init_files.after_load") else "false"

    def vunit_user_init(self, _args):
        option = "yavs.init_file.gui"
        file_name = self._config.sim_options.get(option, None)
        if file_name is not None and self._source(file_name, option):
            return "true"
        return "0"

    def _suite_done(self):
        try:
            return "test_suite_done" in Path(self._results).read_text(encoding="utf-8").splitlines()
        except OSError:
            return False

    def vunit_run(self, _args):
        self._source_all("yavs.init_files.before_run")
        ok, _, error = self.hook.exec("run")
        if not ok:
            self.hook.print(error)
            return "true"
        if not self._suite_done():
            self.hook.print()
            self.hook.print("Test Run Failed!")
            self.hook.print()
            return "true"
        return "false"

    def vunit_compile(self, _args):
        self.hook.print(f"Re-compiling using command {' '.join(self._compile_cmd)}")
        # stderr goes with stdout: the return code alone decides, as in VUnit
        with subprocess.Popen(
            self._compile_cmd,
            cwd=self._cwd,
            stdout=subprocess.PIPE,
            stderr=subprocess.STDOUT,
            text=True,
            bufsize=1,
            env={k: v for k, v in os.environ.items() if k != HOOK_VAR},
        ) as proc:
            for line in proc.stdout:
                self.hook.print(line.rstrip("\n"))
        if proc.returncode != 0:
            self.hook.print("Re-compile failed")
            self.hook.print("child process exited abnormally")
            return "true"
        self.hook.print("Re-compile finished")
        return "false"

    def vunit_restart(self, _args):
        if self.vunit_compile("") != "false":
            return ""
        ok, _, error = self.hook.reload([])
        if not ok:
            self.hook.print(error)
            return "true"
        return self.vunit_run("")

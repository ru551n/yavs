# This Source Code Form is subject to the terms of the Mozilla Public
# License, v. 2.0. If a copy of the MPL was not distributed with this file,
# You can obtain one at http://mozilla.org/MPL/2.0/.
#
# Copyright (c) 2014-2026, Lars Asplund lars.anders.asplund@gmail.com

"""
The yavs simulator interface for VUnit, added as a package:

    vu = VUnit.from_argv()
    vu.add_package("vunit-yavs", allow_setup=True)  # before any other call
    ...

and selected with VUNIT_SIMULATOR=yavs.
"""

import argparse
import atexit
import logging
import re
import subprocess
import sys
import threading
import warnings
from functools import total_ordering
import os
from os import environ, makedirs
from pathlib import Path

from vunit.exceptions import CompileError
from vunit.ostools import Process
from vunit import sim_if as vunit_sim_if
from vunit.color_printer import NO_COLOR_PRINTER
from vunit.sim_if import ListOfStringOption, Option, SimulatorInterface, StringOption, check_executable, hooks
from vunit.vhdl_standard import VHDL

from .console import START, VunitCommands
from .tokens import TokenServer

LOGGER = logging.getLogger(__name__)

# The attach port of the editor extension's "Attach to yavs" configuration
DEFAULT_DEBUG_PORT = 4711
DAP_LISTENING = "yavs: DAP listening on "
# The file names of one yavs analyze call: well below the Windows command line limit of 32767 characters
MAX_BATCH_CHARS = 20000


class PositiveIntegerOption(Option):
    """
    Must be an integer of at least 1
    """

    def validate(self, value):
        if isinstance(value, bool) or not isinstance(value, int) or value < 1:
            raise ValueError(f"Option {self.name!r} must be an integer of at least 1. Got {value!r}")


def _debug_port(text):
    """argparse type of --yavs-debug: a TCP port, 0 for a free one"""
    if not text.isdigit() or int(text) > 65535:
        raise argparse.ArgumentTypeError(
            f"expected a port number 0..65535, got {text!r}. Write the port as --yavs-debug=PORT"
            " (a test pattern right after a bare --yavs-debug is read as its port: put patterns first)"
        )
    return int(text)


def _max_threads(text):
    """argparse type of --yavs-max-threads"""
    if not text.isdigit() or int(text) < 1:
        raise argparse.ArgumentTypeError(f"expected an integer of at least 1, got {text!r}")
    return int(text)


def _exit_with_failure():
    """At exit: end the process with status 1, which VUnit's own exit cannot be told to do from post_run"""
    sys.stdout.flush()
    sys.stderr.flush()
    os._exit(1)  # pylint: disable=protected-access


def _percentage(text):
    """argparse type of --yavs-cov-fail-under: 0 to 100, kept as written for yavs to check"""
    try:
        valid = 0 <= float(text) <= 100
    except ValueError:
        valid = False
    if not valid:
        raise argparse.ArgumentTypeError(f"expected a percentage from 0 to 100, got {text!r}")
    return text


class CoverageArgs:  # pylint: disable=too-few-public-methods
    """
    The coverage command line arguments: --yavs-cov, --yavs-cov-html, --yavs-cov-fail-under
    """

    def __init__(self, enabled=False, html=None, fail_under=None):
        self.enabled = enabled
        self.html = html
        self.fail_under = fail_under


class YavsInterface(SimulatorInterface):
    """
    Interface for the yavs simulator
    """

    name = "yavs"
    executable = environ.get("YAVS", "yavs")
    supports_gui_flag = True
    # yavs-gui drives the run over a debug protocol, so no terminal colour codes
    supports_colors_in_gui = False

    compile_options = [
        ListOfStringOption("yavs.global_flags"),
        ListOfStringOption("yavs.a_flags"),
    ]

    sim_options = [
        ListOfStringOption("yavs.global_flags"),
        ListOfStringOption("yavs.elab_flags"),
        ListOfStringOption("yavs.sim_flags"),
        PositiveIntegerOption("yavs.max_threads"),
        ListOfStringOption("yavs.cov_kind"),
        # Files of console commands, sourced at the points VUnit sources its init files
        ListOfStringOption("yavs.init_files.after_load"),
        ListOfStringOption("yavs.init_files.before_run"),
        StringOption("yavs.init_file.gui"),
    ]

    # For VUnit packages that load a native library: their VHDL calls it through VHPIDIRECT
    # foreign subprograms, and native_library_hooks makes yavs load it
    foreign_language_interface = "VHPIDIRECT"

    @staticmethod
    def supports_coverage():
        """
        Returns True when the simulator supports coverage
        """
        return True

    @staticmethod
    def native_library_hooks(library_file):
        """
        The simulator hooks loading a native library: --load on every 'yavs run'
        """
        return {"run_flags": lambda simulator_interface: [f"--load={library_file}"]}

    @staticmethod
    def add_arguments(parser):
        """
        Add the yavs command line arguments
        """
        group = parser.add_argument_group("yavs", description="yavs specific flags")
        group.add_argument(
            "--yavs-debug",
            nargs="?",
            const=DEFAULT_DEBUG_PORT,
            default=None,
            type=_debug_port,
            metavar="PORT",
            help=(
                "Run the one selected test under the yavs debugger: yavs waits for a Debug Adapter Protocol "
                f"client on 127.0.0.1:PORT (default {DEFAULT_DEBUG_PORT}, 0 picks a free port) before the "
                "simulation starts. Write the port as --yavs-debug=PORT."
            ),
        )
        group.add_argument(
            "--yavs-interactive",
            action="store_true",
            default=False,
            help=(
                "Run the one selected test at the yavs console (yavs run --interactive): a yavs> prompt at time 0, "
                "with the yavs commands and vunit_help, vunit_load, vunit_run, vunit_compile, vunit_restart and "
                "vunit_user_init."
            ),
        )
        group.add_argument(
            "--yavs-surfer",
            nargs="?",
            const="",
            default=None,
            metavar="HOST:PORT",
            help=(
                "Show the waves of the one selected test in Surfer while it runs (yavs run --surfer): start surfer from "
                "PATH, or with --yavs-surfer=HOST:PORT connect to a running one. Write the address with =."
            ),
        )
        group.add_argument(
            "--yavs-max-threads",
            type=_max_threads,
            default=None,
            metavar="N",
            help="Run each simulation with at most N threads (yavs run --threads=N); "
            "the sim option yavs.max_threads overrides it per test.",
        )
        group.add_argument(
            "--yavs-jobserver",
            action="store_true",
            default=False,
            help="Lend the CPU slots of -p N that no test uses to the running simulations through a token server; "
            "each simulation runs with --threads=N (yavs.max_threads or --yavs-max-threads caps it).",
        )

        group.add_argument(
            "--yavs-cov",
            action="store_true",
            default=False,
            help="Collect coverage in every simulation, as the VUnit option enable_coverage does for the tests "
            "it is set for. Merge and report with results.merge_coverage() in the post_run function.",
        )
        group.add_argument(
            "--yavs-cov-html",
            default=None,
            metavar="DIR",
            help="Write an HTML coverage report into DIR when results.merge_coverage() merges the databases.",
        )
        group.add_argument(
            "--yavs-cov-fail-under",
            type=_percentage,
            default=None,
            metavar="PCT",
            help="Print the coverage report and exit with status 1 when results.merge_coverage() finds the total "
            "coverage below PCT percent.",
        )

    @classmethod
    def from_args(cls, args, output_path, **kwargs):
        """
        Create instance from args namespace
        """
        # Arguments added by a package are missing from a namespace the run script built itself
        debug_port = getattr(args, "yavs_debug", None)
        interactive = getattr(args, "yavs_interactive", False)
        surfer = getattr(args, "yavs_surfer", None)
        if surfer is not None:
            check_surfer_args(args)
        if debug_port is not None:
            check_debug_args(args)
        if interactive:
            check_interactive_args(args, debug_port)

        tokens = None
        if getattr(args, "yavs_jobserver", False) and args.num_threads > 1:
            tokens = TokenServer(args.num_threads)
            atexit.register(tokens.close)

        prefix = cls.find_prefix()
        check_executable("YAVS", prefix, cls.executable)
        cls.check_version(prefix)

        return cls(
            output_path=output_path,
            prefix=prefix,
            gui=args.gui,
            debug_port=debug_port,
            interactive=interactive,
            surfer=surfer,
            max_threads=getattr(args, "yavs_max_threads", None),
            tokens=tokens,
            coverage=CoverageArgs(
                enabled=getattr(args, "yavs_cov", False),
                html=getattr(args, "yavs_cov_html", None),
                fail_under=getattr(args, "yavs_cov_fail_under", None),
            ),
        )

    @classmethod
    def find_prefix_from_path(cls):
        """
        Find first valid yavs toolchain prefix
        """
        return cls.find_toolchain([cls.executable])

    def __init__(
        self, output_path, prefix, *, gui=False, debug_port=None, interactive=False, surfer=None, max_threads=None, tokens=None, coverage=None
    ):  # pylint: disable=too-many-arguments
        SimulatorInterface.__init__(self, output_path, gui)
        self._coverage = coverage or CoverageArgs()
        self._coverage_files = set()  # The database of every simulation run with coverage
        self._prefix = prefix
        self._project = None
        self._debug_port = debug_port
        self._interactive = interactive
        self._surfer = surfer
        self._max_threads = max_threads
        self._debug_lock = threading.Lock()
        self._debugged = None
        self._tokens = tokens  # The token server shared by all simulations of the run

    @classmethod
    def _get_version_output(cls, prefix):
        """
        Get the output of 'yavs --version'
        """
        return subprocess.check_output([str(Path(prefix) / cls.executable), "--version"]).decode()

    @classmethod
    def determine_version(cls, prefix):
        """
        Determine the yavs version
        """
        return YavsVersion.from_output(cls._get_version_output(prefix))

    @classmethod
    def check_version(cls, prefix):
        """
        Raise an error unless 'yavs --version' works and reports a version implementing the CLI contract
        """
        try:
            version = cls.determine_version(prefix)
        except (OSError, subprocess.CalledProcessError) as exc:
            raise RuntimeError(f"Running '{Path(prefix) / cls.executable} --version' failed: {exc}") from exc

        if not version.is_valid:
            raise RuntimeError(f"Cannot parse the yavs version from {version.version_string!r}")

        if version < YAVS_MIN_VERSION:
            raise RuntimeError(
                f"yavs {version} at {Path(prefix) / cls.executable} is too old for vunit-yavs, which needs yavs "
                f"{YAVS_MIN_VERSION} or later. Install a newer yavs, or point VUNIT_YAVS_PATH at one."
            )

        return version

    @staticmethod
    def has_valid_exit_code():
        """
        Return if the simulation should fail with nonzero exit codes
        """
        return True

    @staticmethod
    def supports_vhpi():
        """
        Returns True when the simulator supports VHPI
        """
        return True

    @classmethod
    def supports_vhdl_call_paths(cls):
        """
        Returns True when this simulator supports VHDL-2019 call paths
        """
        return True

    @classmethod
    def supports_vhdl_package_generics(cls):
        """
        Returns True when this simulator supports VHDL package generics
        """
        return True

    def setup_library_mapping(self, project):
        """
        Setup library mapping. A library is a directory that yavs creates on first analysis,
        and a missing directory is an empty library, so there is nothing to initialise.
        """
        self._project = project

        for library in project.get_libraries():
            makedirs(Path(library.directory).parent, exist_ok=True)

        # yavs records the standard of each file, so 2008 and 2019 files can be mixed
        for source_file in project.get_source_files_in_order():
            if source_file.is_vhdl and source_file.get_vhdl_standard() not in (VHDL.STD_2008, VHDL.STD_2019):
                raise RuntimeError(
                    f"yavs supports VHDL-2008 and VHDL-2019 only, found VHDL-{source_file.get_vhdl_standard()}"
                    f" for {source_file.name}"
                )

    def compile_source_file_command(self, source_file):
        """
        Returns the command to compile a single source_file
        """
        if source_file.is_vhdl:
            return self.compile_vhdl_file_command(source_file)

        LOGGER.error("Unknown file type: %s", source_file.file_type)
        raise CompileError

    def _get_command(self, subcommand, worklib, workpath):
        """
        Get basic yavs command with library mapping
        """
        cmd = [
            str(Path(self._prefix) / self.executable),
            subcommand,
            f"--work={worklib}:{workpath!s}",
        ]

        for library in self._project.get_libraries():
            cmd += [f"--map={library.name}:{library.directory}"]

        # Keep the shared std/ieee units in the output directory, not the user's cache. A --std-cache in the raw
        # flags, which come later, wins (last one wins in yavs).
        cmd += [f"--std-cache={Path(self._output_path).resolve() / 'std_cache'}"]

        if self.use_color:
            cmd += ["--color=always"]

        return cmd

    def compile_source_files(self, project, printer=NO_COLOR_PRINTER, continue_on_error=False, target_files=None):
        """
        Analyze consecutive files of one library, standard and flags in one yavs call: the dependencies are read
        once. A call that fails leaves its files and all later ones to VUnit's own loop, file by file, which reports
        the failing file and skips what depends on it.
        """
        if target_files is None:
            files = project.get_files_in_compile_order()
        else:
            files = project.get_minimal_file_set_in_compile_order(target_files)
        batched = False
        for batch in _batches(files):
            cmd = self.compile_vhdl_file_command(batch[0])[:-1] + [source_file.name for source_file in batch]
            try:
                output = vunit_sim_if.check_output(cmd, env=self.get_env())
            except subprocess.CalledProcessError:
                break
            printer.write(output)
            for source_file in batch:
                printer.write(f"Compiling into {source_file.library.name}: {source_file.name} passed\n")
                project.update(source_file)
            batched = True
        if batched and not project.get_files_in_compile_order():
            printer.write("Compile passed\n", fg="gi")
            return
        super().compile_source_files(project, printer, continue_on_error, target_files)

    def compile_vhdl_file_command(self, source_file):
        """
        Returns the command to compile a VHDL file
        """
        cmd = self._get_command("analyze", source_file.library.name, source_file.library.directory)
        cmd.insert(2, f"--std={source_file.get_vhdl_standard()}")
        cmd += source_file.compile_options.get("yavs.global_flags", [])
        cmd += source_file.compile_options.get("yavs.a_flags", [])
        cmd += [source_file.name]
        return cmd

    def simulate(self, output_path, test_suite_name, config, elaborate_only):  # pylint: disable=too-many-locals
        """
        Simulate with entity as top level using generics
        """
        interactive = self._interactive and not elaborate_only
        debug = (self._debug_port is not None or interactive) and not elaborate_only
        surfer = self._surfer is not None and not elaborate_only
        if debug or surfer:
            self._take_debug_session(test_suite_name)

        libdir = self._project.get_library(config.library_name).directory
        # No --std: yavs elaborates with the newest standard recorded in the design closure
        cmd = self._get_command("elab" if elaborate_only else "run", config.library_name, libdir)
        script_path = Path(output_path) / self.name
        makedirs(script_path, exist_ok=True)

        if not elaborate_only and config.sim_options.get("disable_ieee_warnings", False):
            cmd += ["--ieee-warnings=off"]

        # The cap goes first, so --threads in the raw flags below wins (last one wins); a DAP run is one thread.
        # With a token server every simulation may use all -p N slots, on tokens.
        max_threads = config.sim_options.get("yavs.max_threads", self._max_threads)
        if max_threads is None and self._tokens is not None:
            max_threads = self._tokens.num_threads
        if max_threads is not None and not elaborate_only and not debug:
            cmd += [f"--threads={max_threads}"]

        if not elaborate_only and (self._coverage.enabled or config.sim_options.get("enable_coverage", False)):
            if debug:
                raise RuntimeError(
                    f"{'--yavs-interactive' if interactive else '--yavs-debug'} and coverage cannot be combined: "
                    "yavs does not collect coverage under the debugger. "
                    "Drop --yavs-cov and the enable_coverage option, or the debug flag."
                )
            database = script_path / "coverage.ycov"
            cmd += [f"--cov={database}", f"--cov-test={test_suite_name}"]
            kinds = config.sim_options.get("yavs.cov_kind", [])
            if kinds:
                cmd += [f"--cov-kind={','.join(kinds)}"]
            self._coverage_files.add(str(database))

        cmd += config.sim_options.get("yavs.global_flags", [])
        cmd += config.sim_options.get("yavs.elab_flags", [])
        cmd += hooks.get_flags(self, "elab_flags")

        if debug:
            cmd += source_map_flags(self._project)
        if surfer:
            # The waves of the test next to its other output
            cmd += [f"--wave={script_path / 'wave.fst'}", f"--surfer={self._surfer}" if self._surfer else "--surfer"]

        if not elaborate_only:
            cmd += config.sim_options.get("yavs.sim_flags", [])
            cmd += hooks.get_flags(self, "run_flags")
            # Last one wins in yavs, so this overrides any --exit-severity in yavs.sim_flags
            cmd += [f"--exit-severity={config.vhdl_assert_stop_level}"]

        if config.vhdl_configuration_name is not None:
            cmd += [config.vhdl_configuration_name]
        else:
            cmd += [f"{config.entity_name}.{config.architecture_name}"]

        for name, value in config.generics.items():
            cmd += [f"-g{name}={value}"]

        print(" ".join([f"'{word}'" if " " in word else word for word in cmd]))

        # Everything after the subcommand goes through a response file. --gui and --dap stay on the command
        # line, so the run has exactly the arguments of the plain run.
        response_file = write_response_file(script_path / "args.rsp", cmd[2:])
        mode_flags = ["--gui"] if self._gui and not elaborate_only else []
        if debug and not interactive:
            mode_flags = [f"--dap={self._debug_port}"]
        if interactive:
            mode_flags = ["--interactive", f"--do={START}"]

        env = hooks.get_run_env(self)
        # The VUnit commands of the console, in the debug console of an editor too
        commands = VunitCommands(output_path, config) if debug else None
        if commands is not None:
            env = commands.hook.env(env)
        client = None
        if self._tokens is not None and not elaborate_only:
            client = self._tokens.add(max_threads - 1)
            env = self._tokens.env(client, environ if env is None else env)

        if interactive:
            try:
                # The terminal is yavs's: no capture of its output
                return subprocess.call(cmd[:2] + mode_flags + [f"@{response_file}"], env=env) == 0
            finally:
                commands.hook.close()

        status = True
        try:
            proc = Process(cmd[:2] + mode_flags + [f"@{response_file}"], env=env)
            proc.consume_output(self._debug_output(test_suite_name) if debug else print)
        except Process.NonZeroExitCode:
            status = False
        finally:
            if client is not None:
                self._tokens.remove(client)
            if commands is not None:
                commands.hook.close()

        return status

    def merge_coverage(self, file_name, args=None):
        """
        Merge the coverage databases of all simulations into file_name (".ycov" is added unless it ends so).

        args are further arguments for 'yavs cov report', which runs on the merged database when there are
        any, or --yavs-cov-html or --yavs-cov-fail-under are given. A report that fails, for example a total
        below --yavs-cov-fail-under, makes the run exit with status 1 once VUnit has written its reports.
        """
        databases = []
        for database in sorted(self._coverage_files):
            if Path(database).exists():
                databases.append(database)
            else:
                LOGGER.warning("Missing coverage database: %s", database)
        if not databases:
            raise RuntimeError(
                "There is no coverage database to merge: no simulation ran with coverage. "
                "Set the sim option enable_coverage or pass --yavs-cov."
            )

        merged = Path(file_name)
        if merged.suffix != ".ycov":
            merged = merged.with_name(merged.name + ".ycov")
        yavs = str(Path(self._prefix) / self.executable)

        print(f"Merging {len(databases)} coverage databases into {merged}...")
        subprocess.run([yavs, "cov", "merge", "-o", str(merged), *databases], check=True)

        report = list(args or [])
        if self._coverage.html is not None:
            report += ["--html", self._coverage.html]
        if self._coverage.fail_under is not None:
            report += [f"--fail-under={self._coverage.fail_under}"]
        if report and subprocess.run([yavs, "cov", "report", str(merged), *report], check=False).returncode != 0:
            # post_run runs before VUnit writes the JUnit report, which an exception here would skip
            atexit.register(_exit_with_failure)

    def _take_debug_session(self, test_suite_name):
        """
        Give the debug session to the first simulation; every later one is an error before yavs starts
        """
        with self._debug_lock:
            if self._debugged is None:
                self._debugged = test_suite_name
                return
            first = self._debugged
        flag = "--yavs-interactive" if self._interactive else "--yavs-debug"
        if self._surfer is not None and not self._interactive and self._debug_port is None:
            flag = "--yavs-surfer"
        example = "--yavs-debug=PORT" if flag == "--yavs-debug" else flag
        raise RuntimeError(
            f"{flag} runs one simulation, but {test_suite_name} would start after {first}.\n"
            "Cause: the test patterns select more than one test (or one test in several configurations).\n"
            f"Fix: name exactly one test, for example 'python run.py {example} lib.tb.test_name' "
            "(see 'python run.py --list')."
        )

    @staticmethod
    def _debug_output(test_suite_name):
        """
        Return an output callback that also shows where to attach on the terminal, since the test runner
        only shows a test's output when it fails or with -v.
        """

        def callback(line):
            print(line)
            if line.startswith(DAP_LISTENING):
                address = line[len(DAP_LISTENING) :].strip()
                sys.__stdout__.write(f"Debugging {test_suite_name}: attach a DAP client to {address}\n")
                sys.__stdout__.flush()

        return callback


def source_map_flags(project):
    """
    Flags that make a debug session show the user's files where the project compiles preprocessed copies
    """
    return [
        f"--source-map={Path(source_file.name).absolute()}={Path(str(source_file.original_name)).absolute()}"
        for source_file in project.get_source_files_in_order()
        if str(source_file.original_name) != source_file.name
    ]


def check_debug_args(args):
    """
    Raise an error for arguments that cannot be combined with --yavs-debug
    """
    if getattr(args, "gui", False):
        raise RuntimeError(
            "--yavs-debug and --gui cannot be combined: both drive the run over the debug protocol. Drop one of them."
        )
    if getattr(args, "elaborate", False):
        raise RuntimeError(
            "--yavs-debug and --elaborate cannot be combined: --elaborate only elaborates, so there is no run to debug. "
            "Drop one of them."
        )
    if getattr(args, "num_threads", 1) != 1:
        raise RuntimeError(
            f"--yavs-debug runs one test, but -p {args.num_threads} asks for parallel simulations. "
            "Drop -p (or use -p 1) and name one test."
        )


def check_interactive_args(args, debug_port):
    """
    Raise an error for arguments that cannot be combined with --yavs-interactive
    """
    if debug_port is not None:
        raise RuntimeError(
            "--yavs-interactive and --yavs-debug cannot be combined: the terminal or a debug client drives the run. "
            "Drop one of them."
        )
    if getattr(args, "gui", False):
        raise RuntimeError("--yavs-interactive and --gui cannot be combined. Drop one of them.")
    if getattr(args, "elaborate", False):
        raise RuntimeError(
            "--yavs-interactive and --elaborate cannot be combined: --elaborate only elaborates, so there is no run. "
            "Drop one of them."
        )
    if getattr(args, "num_threads", 1) != 1:
        raise RuntimeError(
            f"--yavs-interactive runs one test, but -p {args.num_threads} asks for parallel simulations. "
            "Drop -p (or use -p 1) and name one test."
        )


def _batches(source_files):
    """
    The runs of two or more consecutive VHDL files that can share a yavs call, each short enough for a command line
    on any platform. Single files stay with VUnit's own loop. Stops at a file that is not VHDL: order must hold.
    """
    runs = []
    key = None
    length = 0
    for source_file in source_files:
        if not source_file.is_vhdl:
            break
        file_key = (
            source_file.library.name,
            source_file.get_vhdl_standard(),
            tuple(source_file.compile_options.get("yavs.global_flags", [])),
            tuple(source_file.compile_options.get("yavs.a_flags", [])),
        )
        length += len(source_file.name) + 1
        if file_key != key or length > MAX_BATCH_CHARS:
            runs.append([])
            key = file_key
            length = len(source_file.name) + 1
        runs[-1].append(source_file)
    return [run for run in runs if len(run) > 1]


def check_surfer_args(args):
    """
    Raise an error for arguments that cannot be combined with --yavs-surfer
    """
    if getattr(args, "num_threads", 1) != 1:
        raise RuntimeError(
            f"--yavs-surfer shows one test, but -p {args.num_threads} asks for parallel simulations. "
            "Drop -p (or use -p 1) and name one test."
        )


def write_response_file(file_name, args):
    """
    Write args to a yavs response file, one argument per line taken verbatim, and return its name.

    A response file keeps long generics such as runner_cfg clear of the Windows command line length limit
    and of its quoting rules.
    """
    for arg in args:
        if "\n" in arg or "\r" in arg:
            raise ValueError(f"yavs arguments cannot contain line breaks: {arg!r}")

    with open(file_name, "w", encoding="utf-8", newline="\n") as fptr:
        fptr.write("".join(arg + "\n" for arg in args))
    return file_name


@total_ordering
class YavsVersion:
    """yavs version number."""

    _VERSION_RE = re.compile(r"([0-9]+)\.([0-9]+)(?:\.([0-9]+))?")

    def __init__(self, version_string):
        self.version_string = version_string
        self._version = None
        match = self._VERSION_RE.match(version_string)
        if not match:
            warnings.warn(
                f"Cannot parse yavs version: {version_string!r}; version is unknown",
                RuntimeWarning,
                stacklevel=2,
            )
            return

        self._version = (int(match.group(1)), int(match.group(2)), int(match.group(3) or 0))

    @classmethod
    def from_output(cls, output):
        """Extract the yavs version from 'yavs --version' output, or return an unknown version."""
        match = cls._VERSION_RE.search(output)
        return cls(match.group(0) if match else output)

    @property
    def is_valid(self):
        return self._version is not None

    def __eq__(self, other):
        if isinstance(other, YavsVersion):
            if not self.is_valid or not other.is_valid:
                raise ValueError("Cannot compare unknown yavs versions")
            return self._version == other._version  # pylint: disable=protected-access
        return NotImplemented

    def __lt__(self, other):
        if isinstance(other, YavsVersion):
            if not self.is_valid or not other.is_valid:
                raise ValueError("Cannot order unknown yavs versions")
            return self._version < other._version  # pylint: disable=protected-access
        return NotImplemented

    def __str__(self):
        return ".".join(str(component) for component in self._version) if self.is_valid else "unknown"

    def __repr__(self):
        return f"YavsVersion({self.version_string!r})"


# The first yavs with what this package uses (--dap, response files, YAVS_TOKENS). Raise it with the CLI contract it relies on.
YAVS_MIN_VERSION = YavsVersion("0.11.0")

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
from os import environ, makedirs
from pathlib import Path

from vunit.exceptions import CompileError
from vunit.ostools import Process
from vunit.sim_if import ListOfStringOption, Option, SimulatorInterface, check_executable, hooks
from vunit.vhdl_standard import VHDL

from .jobserver import Jobserver

LOGGER = logging.getLogger(__name__)

# The attach port of the editor extension's "Attach to yavs" configuration
DEFAULT_DEBUG_PORT = 4711
DAP_LISTENING = "yavs: DAP listening on "


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
    ]

    # For VUnit packages that load a native library: their VHDL calls it through VHPIDIRECT
    # foreign subprograms, and native_library_hooks makes yavs load it
    foreign_language_interface = "VHPIDIRECT"

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
            help="Lend the CPU slots of -p N that no test uses to the running simulations through a jobserver "
            "(POSIX only); yavs.max_threads caps each test.",
        )

    @classmethod
    def from_args(cls, args, output_path, **kwargs):
        """
        Create instance from args namespace
        """
        # Arguments added by a package are missing from a namespace the run script built itself
        debug_port = getattr(args, "yavs_debug", None)
        if debug_port is not None:
            check_debug_args(args)

        jobserver = None
        if getattr(args, "yavs_jobserver", False):
            if sys.platform == "win32":
                raise RuntimeError(
                    "--yavs-jobserver needs POSIX named pipes, which Windows does not have. Drop --yavs-jobserver."
                )
            if args.num_threads > 1:
                jobserver = Jobserver(args.num_threads)
                atexit.register(jobserver.close)

        prefix = cls.find_prefix()
        check_executable("YAVS", prefix, cls.executable)
        cls.check_version(prefix)

        return cls(
            output_path=output_path,
            prefix=prefix,
            gui=args.gui,
            debug_port=debug_port,
            max_threads=getattr(args, "yavs_max_threads", None),
            jobserver=jobserver,
        )

    @classmethod
    def find_prefix_from_path(cls):
        """
        Find first valid yavs toolchain prefix
        """
        return cls.find_toolchain([cls.executable])

    def __init__(
        self, output_path, prefix, *, gui=False, debug_port=None, max_threads=None, jobserver=None
    ):  # pylint: disable=too-many-arguments
        SimulatorInterface.__init__(self, output_path, gui)
        self._prefix = prefix
        self._project = None
        self._debug_port = debug_port
        self._max_threads = max_threads
        self._debug_lock = threading.Lock()
        self._debugged = None
        self._jobserver = jobserver  # Shared by all simulations of the run

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
        debug = self._debug_port is not None and not elaborate_only
        if debug:
            self._take_debug_session(test_suite_name)

        libdir = self._project.get_library(config.library_name).directory
        # No --std: yavs elaborates with the newest standard recorded in the design closure
        cmd = self._get_command("elab" if elaborate_only else "run", config.library_name, libdir)
        script_path = Path(output_path) / self.name
        makedirs(script_path, exist_ok=True)

        if not elaborate_only and config.sim_options.get("disable_ieee_warnings", False):
            cmd += ["--ieee-warnings=off"]

        # The cap goes first, so --threads in the raw flags below wins (last one wins); a DAP run is one thread
        max_threads = config.sim_options.get("yavs.max_threads", self._max_threads)
        if max_threads is not None and not elaborate_only and not debug:
            cmd += [f"--threads={max_threads}"]

        cmd += config.sim_options.get("yavs.global_flags", [])
        cmd += config.sim_options.get("yavs.elab_flags", [])
        cmd += hooks.get_flags(self, "elab_flags")

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
        if debug:
            mode_flags = [f"--dap={self._debug_port}"]

        env = hooks.get_run_env(self)
        client = None
        if self._jobserver is not None and not elaborate_only:
            client = self._jobserver.add(None if max_threads is None else max_threads - 1)
            env = self._jobserver.env(client, env)

        status = True
        try:
            proc = Process(cmd[:2] + mode_flags + [f"@{response_file}"], env=env)
            proc.consume_output(self._debug_output(test_suite_name) if debug else print)
        except Process.NonZeroExitCode:
            status = False
        finally:
            if client is not None:
                self._jobserver.remove(client)

        return status

    def _take_debug_session(self, test_suite_name):
        """
        Give the debug session to the first simulation; every later one is an error before yavs starts
        """
        with self._debug_lock:
            if self._debugged is None:
                self._debugged = test_suite_name
                return
            first = self._debugged
        raise RuntimeError(
            f"--yavs-debug runs one simulation, but {test_suite_name} would start after {first}.\n"
            "Cause: the test patterns select more than one test (or one test in several configurations).\n"
            "Fix: name exactly one test, for example 'python run.py --yavs-debug=PORT lib.tb.test_name' "
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


# The first yavs with what this package uses (--dap, response files). Raise it with the CLI contract it relies on.
YAVS_MIN_VERSION = YavsVersion("0.10.0")

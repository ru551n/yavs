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
from concurrent.futures import FIRST_COMPLETED, ThreadPoolExecutor, wait
from functools import total_ordering
import os
from multiprocessing import cpu_count
from os import environ, makedirs
from pathlib import Path

from vunit.color_printer import NO_COLOR_PRINTER
from vunit.exceptions import CompileError
from vunit.ostools import Process, simplify_path
from vunit import sim_if as vunit_sim_if
from vunit.color_printer import NO_COLOR_PRINTER
from vunit.sim_if import BooleanOption, ListOfStringOption, Option, SimulatorInterface, StringOption, check_executable, hooks
from vunit.vhdl_standard import VHDL

from .console import START, VunitCommands
from .tokens import TokenServer

LOGGER = logging.getLogger(__name__)

# The attach port of the editor extension's "Attach to yavs" configuration
DEFAULT_DEBUG_PORT = 4711
DAP_LISTENING = "yavs: DAP listening on "
# The file names of one yavs analyze call: well below the Windows command line limit of 32767 characters
MAX_BATCH_CHARS = 20000
# The library clauses of a VHDL file, after its comments are removed
LIBRARY_CLAUSE = re.compile(r"\blibrary\s+(\w+(?:\s*,\s*\w+)*)\s*;", re.IGNORECASE)
# With a token server a simulation runs with --threads=auto unless told otherwise: it uses tokens only
# while its threads gain, at most this many threads (the cap of yavs's own --threads=auto, #884)
AUTO_MAX_THREADS = 8


class ThreadsOption(Option):
    """
    Must be an integer of at least 1, or "auto"
    """

    def validate(self, value):
        if value != "auto" and (isinstance(value, bool) or not isinstance(value, int) or value < 1):
            raise ValueError(f"Option {self.name!r} must be an integer of at least 1 or \"auto\". Got {value!r}")


class TierCompileOption(Option):
    """
    Must be "wait" or "background" (yavs run --tier-compile)
    """

    def validate(self, value):
        if value not in ("wait", "background"):
            raise ValueError(f"Option {self.name!r} must be \"wait\" or \"background\". Got {value!r}")


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
    if text == "auto":
        return text
    if not text.isdigit() or int(text) < 1:
        raise argparse.ArgumentTypeError(f"expected an integer of at least 1 or auto, got {text!r}")
    return int(text)


def _jobs(text):
    """argparse type of -j/--jobs"""
    if not text.isdigit() or int(text) < 1:
        raise argparse.ArgumentTypeError(
            f"expected an integer of at least 1, got {text!r}; put test names before a bare -j (one call per CPU)"
        )
    return int(text)


def refuse(message):
    """
    Stop the run script with `message` alone: a wrong command line or setup needs its cause and fix, not a traceback.
    VUnit's main turns this exit into status 1.
    """
    print(f"error: {message}", file=sys.stderr, flush=True)
    sys.exit(1)


def _max_threads_from_environment():
    """VUNIT_YAVS_MAX_THREADS, for a run script that parses its own command line and so has no --yavs-max-threads"""
    text = environ.get("VUNIT_YAVS_MAX_THREADS")
    if text is None:
        return None
    try:
        return _max_threads(text)
    except argparse.ArgumentTypeError as exc:
        return refuse(f"VUNIT_YAVS_MAX_THREADS: {exc}")


def _exit_with_failure():
    """At exit: end the process with status 1, which VUnit's own exit cannot be told to do from post_run"""
    sys.stdout.flush()
    sys.stderr.flush()
    os._exit(1)  # pylint: disable=protected-access


# The kinds yavs counts; test_real_yavs.py checks this list against the yavs in use
_COV_KINDS = ("statement", "branch", "toggle", "condition", "expression", "combination", "fsm")


# The letters of 'yavs run --cov=LETTERS': test_real_yavs.py checks this table against the yavs in use
_COV_LETTERS = {
    "s": "statement",
    "b": "branch",
    "c": "condition",
    "e": "expression",
    "t": "toggle",
    "f": "fsm",
    "a": "assert",
    "m": "combination",
}


def _cov_letters_text():
    """The table of coverage kinds, letter and word, for the messages that name them"""
    return "\n".join(f"  {letter}  {word}" for letter, word in _COV_LETTERS.items())


def _cov_letters(text):
    """argparse type of --yavs-cov: kind letters, or empty for a bare flag that run_script then refuses"""
    unknown = sorted({letter for letter in text if letter not in _COV_LETTERS})
    if unknown:
        raise argparse.ArgumentTypeError(
            f"{', '.join(map(repr, unknown))} in --yavs-cov={text} is not a coverage kind letter; the letters are:\n"
            f"{_cov_letters_text()}\nWrite the letters with =, as --yavs-cov=sbt; words go in --yavs-cov-kind=LIST"
        )
    return text


# The exports of 'yavs cov report': the flag and what it writes
_COV_EXPORTS = {"cobertura": "Cobertura XML", "json": "JSON", "lcov": "LCOV", "sonarqube": "SonarQube generic coverage"}


def _coverage_requested(args):
    """Whether --yavs-cov or --yavs-cov-kind turns coverage on, as yavs run --cov and --cov-kind do"""
    return getattr(args, "yavs_cov", None) is not None or bool(getattr(args, "yavs_cov_kind", None))


def _plain_percentage(text):
    """argparse type of --yavs-cov-below: 0 to 100, kept as written for yavs to check"""
    try:
        valid = 0 <= float(text) <= 100
    except ValueError:
        valid = False
    if not valid:
        raise argparse.ArgumentTypeError(f"expected a percentage from 0 to 100, got {text!r}")
    return text


def _percentage(text):
    """argparse type of --yavs-cov-fail-under: PCT or KIND:PCT, comma separated, kept as written for yavs to check"""
    for part in text.split(","):
        kind, colon, number = part.rpartition(":")
        if colon and kind not in _COV_KINDS:
            raise argparse.ArgumentTypeError(f"{kind!r} is not a coverage kind; the kinds are {', '.join(_COV_KINDS)}")
        try:
            valid = 0 <= float(number) <= 100
        except ValueError:
            valid = False
        if not valid:
            raise argparse.ArgumentTypeError(
                f"expected a percentage from 0 to 100 or KIND:PCT such as statement:90,branch:80, got {text!r}"
            )
    return text


class CoverageArgs:  # pylint: disable=too-few-public-methods
    """
    The coverage command line arguments: --yavs-cov (the kind letters), --yavs-cov-kind, --yavs-cov-html, --yavs-cov-fail-under, --yavs-cov-below,
    --yavs-cov-detailed, --yavs-cov-embed-sources, --yavs-cov-exclude, --yavs-cov-config
    """

    def __init__(  # pylint: disable=too-many-arguments
        self,
        enabled=False,
        html=None,
        fail_under=None,
        below=None,
        detailed=False,
        embed_sources=False,
        exclude=None,
        config=None,
        exports=None,
        kinds=None,
        letters=None,
    ):
        self.enabled = enabled
        self.kinds = kinds
        self.letters = letters
        self.html = html
        self.fail_under = fail_under
        self.below = below
        self.detailed = detailed
        self.embed_sources = embed_sources
        self.exclude = exclude or []
        self.config = config
        self.exports = exports or {}


class YavsInterface(SimulatorInterface):
    """
    Interface for the yavs simulator
    """

    name = "yavs"
    executable = environ.get("YAVS", "yavs")
    # yavs locks a library for each analyze call and takes many files in one call, so a runner with
    # --parallel-compile and --batch-compile may use both. The file names of a batch go on the command
    # line, not in a file list (batch_file_list_arguments stays None): the runner writes such a list in
    # the locale encoding and yavs reads UTF-8 (#114).
    supports_concurrent_compiles_into_library = True
    supports_batch_compile = True
    batch_file_list_arguments = None

    compile_options = [
        ListOfStringOption("yavs.global_flags"),
        ListOfStringOption("yavs.a_flags"),
    ]

    sim_options = [
        ListOfStringOption("yavs.global_flags"),
        ListOfStringOption("yavs.elab_flags"),
        ListOfStringOption("yavs.sim_flags"),
        ThreadsOption("yavs.max_threads"),
        TierCompileOption("yavs.tier_compile"),
        ListOfStringOption("yavs.cov_kind"),
        BooleanOption("yavs.cov_embed_sources"),
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
    def native_library_hooks(library_file, args=None):
        """
        The simulator hooks loading a native library: --load on every 'yavs elab' and 'yavs run' (a signal's
        initial value may call it, so elaborating alone needs it too).
        args names the layout its functions take their arguments in (`"fat-pointer"`), default the usual one.
        """
        load = f"--load={library_file}" + (f":args={args}" if args else "")
        return {"elab_flags": lambda simulator_interface: [load]}

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
            "-j",
            "--jobs",
            dest="yavs_jobs",
            nargs="?",
            const=cpu_count(),
            type=_jobs,
            default=None,
            metavar="N",
            help="Compile with up to N yavs analyze calls at once, in the project's dependency order; calls into "
            "one library wait for each other. A bare -j is one per CPU. Default: one call at a time. "
            "Independent of -p, which counts simulations.",
        )
        group.add_argument(
            "--yavs-max-threads",
            type=_max_threads,
            default=None,
            metavar="N",
            help="Run each simulation with at most N threads (yavs run --threads=N), or with --threads=auto "
            "(N=auto: threads only while they make the simulation faster); "
            "the sim option yavs.max_threads overrides it per test. Default: the environment variable "
            "VUNIT_YAVS_MAX_THREADS, for run scripts with a command line of their own.",
        )
        group.add_argument(
            "--yavs-jobserver",
            action="store_true",
            default=False,
            help="Lend the CPU slots of -p N that no test uses to the running simulations through a token server; "
            "each simulation runs with --threads=auto (yavs.max_threads or --yavs-max-threads sets another value).",
        )

        group.add_argument(
            "--yavs-lint",
            action="store_true",
            default=False,
            help="After compiling, run yavs lint on every library of the project; a finding of severity error "
            "fails the compile step.",
        )
        group.add_argument(
            "--yavs-cov",
            nargs="?",
            const="",
            default=None,
            type=_cov_letters,
            metavar="LETTERS",
            help="Collect coverage in every simulation, as the VUnit option enable_coverage does for the tests "
            "it is set for, of the kinds the letters name (--yavs-cov=sbt): s statement, b branch, c condition, "
            "e expression, t toggle, f fsm, a assert, m combination. No kind is on by default. Write the letters "
            "with =. Merge and report with results.merge_coverage() in the post_run function.",
        )
        group.add_argument(
            "--yavs-cov-directives",
            choices=["apply", "ignore"],
            default=None,
            help="What yavs does with the '-- yavs coverage exclude' comments in the sources, when analysing, "
            "elaborating and running: apply (yavs's default) or ignore (read none of them).",
        )
        group.add_argument(
            "--yavs-cov-kind",
            default=None,
            metavar="KIND,...",
            help="The kinds of coverage to count, as yavs run --cov-kind (fsm, assert, condition, ...), for the tests "
            "without a yavs.cov_kind sim option; the long form of --yavs-cov=LETTERS, not with it, and it also turns "
            "coverage on.",
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
            "coverage below PCT percent, or a kind below its own (statement:90,branch:80).",
        )
        for flag, what in _COV_EXPORTS.items():
            group.add_argument(
                f"--yavs-cov-{flag}",
                metavar="FILE",
                default=None,
                help=f"Write a {what} report to FILE when results.merge_coverage() reports.",
            )
        group.add_argument(
            "--yavs-cov-embed-sources",
            action="store_true",
            default=False,
            help="Store the source text in every coverage database, so the HTML report needs no checkout. The source "
            "text then travels with the database.",
        )
        group.add_argument(
            "--yavs-cov-below",
            metavar="PCT",
            type=_plain_percentage,
            default=None,
            help="Limit the table in the report of results.merge_coverage() to the instances below PCT percent, "
            "worst first.",
        )
        group.add_argument(
            "--yavs-cov-detailed",
            action="store_true",
            default=False,
            help="The report of results.merge_coverage() also lists what never ran, under each instance.",
        )
        group.add_argument(
            "--yavs-cov-exclude",
            action="append",
            default=None,
            metavar="FILE",
            help="Leave out the items an exclusion file lists when results.merge_coverage() reports (yavs cov report "
            "--exclude). Repeatable: the rules of all files add up.",
        )
        group.add_argument(
            "--yavs-cov-config",
            default=None,
            metavar="FILE",
            help="Read exclusion files, root and kinds from a YAML file when results.merge_coverage() reports "
            "(yavs cov report --config).",
        )

    @classmethod
    def from_args(cls, args, output_path, **kwargs):
        """
        Create instance from args namespace
        """
        if getattr(args, "gui", False):
            refuse(
                "yavs has no graphical debugger, so -g/--gui is not supported. Drop it; to debug a test, "
                "use --yavs-debug and attach a DAP client such as VS Code."
            )
        # Arguments added by a package are missing from a namespace the run script built itself
        if getattr(args, "yavs_cov", None) and getattr(args, "yavs_cov_kind", None):
            refuse("--yavs-cov=LETTERS and --yavs-cov-kind both select the coverage kinds. Give the kinds once.")
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
        # -p 0 means one job per CPU: the test runner resolves it the same way (#1141)
        jobs = (args.num_threads or cpu_count()) if getattr(args, "yavs_jobserver", False) else 1
        if jobs > 1:
            try:
                tokens = TokenServer(jobs)
            except OSError as exc:
                # Tokens only speed a run up: every simulation then runs on one thread (#878)
                LOGGER.warning("--yavs-jobserver: cannot start the token server (%s); simulations use one thread", exc)
            else:
                atexit.register(tokens.close)

        prefix = cls.find_prefix()
        check_executable("YAVS", prefix, cls.executable)
        cls.check_version(prefix)

        return cls(
            output_path=output_path,
            prefix=prefix,
            debug_port=debug_port,
            interactive=interactive,
            surfer=surfer,
            max_threads=getattr(args, "yavs_max_threads", None) or _max_threads_from_environment(),
            tokens=tokens,
            lint=getattr(args, "yavs_lint", False),
            compile_jobs=getattr(args, "yavs_jobs", None) or 1,
            cov_directives=getattr(args, "yavs_cov_directives", None),
            coverage=CoverageArgs(
                enabled=_coverage_requested(args),
                kinds=getattr(args, "yavs_cov_kind", None),
                letters=getattr(args, "yavs_cov", None) or None,
                html=getattr(args, "yavs_cov_html", None),
                fail_under=getattr(args, "yavs_cov_fail_under", None),
                below=getattr(args, "yavs_cov_below", None),
                detailed=getattr(args, "yavs_cov_detailed", False),
                exports={flag: getattr(args, f"yavs_cov_{flag}", None) for flag in _COV_EXPORTS},
                embed_sources=getattr(args, "yavs_cov_embed_sources", False),
                exclude=getattr(args, "yavs_cov_exclude", None),
                config=getattr(args, "yavs_cov_config", None),
            ),
        )

    @classmethod
    def find_prefix_from_path(cls):
        """
        Find first valid yavs toolchain prefix
        """
        return cls.find_toolchain([cls.executable])

    def __init__(
        self,
        output_path,
        prefix,
        *,
        debug_port=None,
        interactive=False,
        surfer=None,
        max_threads=None,
        tokens=None,
        coverage=None,
        lint=False,
        cov_directives=None,
        compile_jobs=1,
    ):  # pylint: disable=too-many-arguments
        SimulatorInterface.__init__(self, output_path, False)
        self._compile_jobs = compile_jobs  # -j: yavs analyze calls at once
        self._lint = lint
        self._cov_directives = cov_directives
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
        Refuse a yavs unless 'yavs --version' works and reports a version implementing the CLI contract
        """
        try:
            version = cls.determine_version(prefix)
        except (OSError, subprocess.CalledProcessError) as exc:
            refuse(f"Running '{Path(prefix) / cls.executable} --version' failed: {exc}")

        if not version.is_valid:
            refuse(f"Cannot parse the yavs version from {version.version_string!r}")

        if version < YAVS_MIN_VERSION:
            refuse(
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
                refuse(
                    f"yavs supports VHDL-2008 and VHDL-2019 only, found VHDL-{source_file.get_vhdl_standard()}"
                    f" for {source_file.name}"
                )

    def compile_project(self, project, printer=NO_COLOR_PRINTER, continue_on_error=False, target_files=None):
        """
        Compile the project, then run yavs lint on it under --yavs-lint
        """
        super().compile_project(project, printer, continue_on_error, target_files)
        if not self._lint:
            return
        # An external library is not this project's, and a library without files has nothing to check
        libraries = [
            library
            for library in project.get_libraries()
            if not library.is_external and (Path(library.directory) / "yavs-lib.toml").exists()
        ]
        if not libraries:
            return
        # Every run reads the same configuration: a bad one is told once, before any run
        settings = [str(Path(self._prefix) / self.executable), "lint", "--list-rules"]
        if subprocess.run(settings, stdout=subprocess.DEVNULL, check=False).returncode != 0:
            raise CompileError
        failed = False
        for library in libraries:
            # The project says which files a library has, not file_list in the configuration
            cmd = self._get_command("lint", library.name, library.directory) + ["--no-file-list"]
            failed |= subprocess.run(cmd, check=False).returncode != 0
        if failed:
            raise CompileError

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

        if self._cov_directives is not None:
            # Before the raw flags, so --coverage-directives in yavs.global_flags wins
            cmd += [f"--coverage-directives={self._cov_directives}"]

        # Keep the shared std/ieee units in the output directory, not the user's cache. A --std-cache in the raw
        # flags, which come later, wins (last one wins in yavs).
        cmd += [f"--std-cache={Path(self._output_path).resolve() / 'std_cache'}"]

        if self.use_color:
            cmd += ["--color=always"]

        return cmd

    def compile_source_files(
        self, project, printer=NO_COLOR_PRINTER, continue_on_error=False, target_files=None, *args, **kwargs
    ):  # pylint: disable=keyword-arg-before-vararg
        """
        Analyze the files in the project's dependency order with up to -j yavs calls at once (_Compile). The output,
        the skipped dependents of a failing file and the stop without continue_on_error are those of VUnit's own loop.

        A runner with --parallel-compile or --batch-compile passes num_threads or batch: it then does all of it.
        """
        if args or kwargs.get("batch") or kwargs.get("num_threads", 1) != 1:
            return super().compile_source_files(project, printer, continue_on_error, target_files, *args, **kwargs)
        graph = project.create_dependency_graph()
        if target_files is None:
            files = project.get_files_in_compile_order(dependency_graph=graph)
        else:
            files = project.get_minimal_file_set_in_compile_order(target_files)
        if not files:
            printer.write("Re-compile not needed\n")
            return None
        groups = _library_groups(project, graph) if self._compile_jobs > 1 else {}
        failures = _Compile(self, project, printer, graph, files, groups).run(self._compile_jobs, continue_on_error)
        if failures:
            printer.write("Compile failed\n", fg="ri")
            if continue_on_error:
                return None
            raise CompileError
        printer.write("Compile passed\n", fg="gi")
        return None

    def _analyze(self, files):
        """Run one yavs analyze call on files (one key) and return (command, passed, output); in a worker thread"""
        try:
            cmd = self.compile_source_file_command(files[0])[:-1] + [source_file.name for source_file in files]
        except CompileError:
            return None, False, f"File type not supported by {self.name!s} simulator\n"
        try:
            return cmd, True, vunit_sim_if.check_output(cmd, env=self.get_env())
        except subprocess.CalledProcessError as err:
            return cmd, False, err.output

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
            if not self._take_debug_session(test_suite_name):
                return False

        libdir = self._project.get_library(config.library_name).directory
        # No --std: yavs elaborates with the newest standard recorded in the design closure
        cmd = self._get_command("elab" if elaborate_only else "run", config.library_name, libdir)
        script_path = Path(output_path) / self.name
        makedirs(script_path, exist_ok=True)

        if not elaborate_only and config.sim_options.get("disable_ieee_warnings", False):
            cmd += ["--ieee-warnings=off"]

        # The cap goes first, so --threads in the raw flags below wins (last one wins); a DAP run is one thread.
        # With a token server every simulation may use the -p N slots, on tokens, while its threads gain.
        max_threads = config.sim_options.get("yavs.max_threads", self._max_threads)
        if max_threads is None and self._tokens is not None:
            max_threads = "auto"
        if max_threads is not None and not elaborate_only and not debug:
            cmd += [f"--threads={max_threads}"]

        # Before the raw flags too: a --tier-compile there wins.
        tier_compile = config.sim_options.get("yavs.tier_compile")
        if tier_compile is not None and not elaborate_only:
            cmd += [f"--tier-compile={tier_compile}"]

        if not elaborate_only and (self._coverage.enabled or config.sim_options.get("enable_coverage", False)):
            if debug:
                print(
                    f"error: {'--yavs-interactive' if interactive else '--yavs-debug'} and coverage cannot be "
                    "combined: yavs does not collect coverage under the debugger. "
                    "Drop --yavs-cov and the enable_coverage option, or the debug flag."
                )
                return False
            database = script_path / "coverage.ycov"
            kinds = config.sim_options.get("yavs.cov_kind", [])
            if kinds:
                cmd += [f"--cov-kind={','.join(kinds)}"]
            elif self._coverage.kinds:
                cmd += [f"--cov-kind={self._coverage.kinds}"]
            elif self._coverage.letters:
                cmd += [f"--cov={self._coverage.letters}"]
            else:
                print(
                    "error: coverage is on, but no coverage kind is selected, and none is on by default.\n"
                    "Fix: give the kinds with --yavs-cov=LETTERS (for example --yavs-cov=sbt), "
                    "--yavs-cov-kind=statement,branch,toggle, or the sim option yavs.cov_kind "
                    "(for example set_sim_option('yavs.cov_kind', ['statement', 'branch', 'toggle'])).\n"
                    f"The kinds:\n{_cov_letters_text()}"
                )
                return False
            cmd += [f"--cov-file={database}", f"--cov-test={test_suite_name}"]
            if self._coverage.embed_sources or config.sim_options.get("yavs.cov_embed_sources", False):
                cmd += ["--cov-embed-sources"]
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

        # Everything after the subcommand goes through a response file. --dap stays on the command
        # line, so the run has exactly the arguments of the plain run.
        response_file = write_response_file(script_path / "args.rsp", cmd[2:])
        mode_flags = []
        if debug and not interactive:
            mode_flags = [f"--dap={self._debug_port}"]
        if interactive:
            mode_flags = ["--interactive", f"--do={START}"]
        elif not elaborate_only:
            # The test runner's Process keeps a pipe to yavs's stdin: when the test runner dies, even by SIGKILL,
            # the pipe closes and yavs ends, instead of waiting for a debugger or finishing alone
            mode_flags += ["--exit-on-stdin-close"]

        env = hooks.get_run_env(self)
        # The VUnit commands of the console, in the debug console of an editor too
        commands = VunitCommands(output_path, config) if debug else None
        if commands is not None:
            env = commands.hook.env(env)
        client = None
        if self._tokens is not None and not elaborate_only:
            cap = min(self._tokens.num_threads, AUTO_MAX_THREADS) if max_threads == "auto" else max_threads
            client = self._tokens.add(cap - 1)
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
        any, or --yavs-cov-html, an export flag, --yavs-cov-fail-under, --yavs-cov-below, --yavs-cov-detailed,
        --yavs-cov-exclude or --yavs-cov-config are given. A report that fails, for example a total below
        --yavs-cov-fail-under, or a merge that fails, makes the run exit with status 1 once VUnit has written its
        reports. Anything else for the report from a run script goes in args: args=["--exclude", file, "--lcov", file].
        """
        databases = []
        for database in sorted(self._coverage_files):
            if Path(database).exists():
                databases.append(database)
            else:
                LOGGER.warning("Missing coverage database: %s", database)
        if not databases:
            refuse(
                "There is no coverage database to merge: no simulation ran with coverage. "
                "Set the sim option enable_coverage or pass --yavs-cov=LETTERS."
            )

        merged = Path(file_name)
        if merged.suffix != ".ycov":
            merged = merged.with_name(merged.name + ".ycov")
        yavs = str(Path(self._prefix) / self.executable)

        print(f"Merging {len(databases)} coverage databases into {merged}...")
        if subprocess.run([yavs, "cov", "merge", "-o", str(merged), *databases], check=False).returncode != 0:
            # yavs said why; as for a failing report, VUnit still writes its reports first
            atexit.register(_exit_with_failure)
            return

        report = list(args or [])
        if self._coverage.html is not None:
            report += ["--html", self._coverage.html]
        for flag, path in self._coverage.exports.items():
            if path is not None:
                report += [f"--{flag}", str(path)]
        if self._coverage.fail_under is not None:
            report += [f"--fail-under={self._coverage.fail_under}"]
        if self._coverage.below is not None:
            report += [f"--below={self._coverage.below}"]
        if self._coverage.detailed:
            report += ["--detailed"]
        for path in self._coverage.exclude:
            report += ["--exclude", str(path)]
        if self._coverage.config is not None:
            report += ["--config", str(self._coverage.config)]
        if report and subprocess.run([yavs, "cov", "report", str(merged), *report], check=False).returncode != 0:
            # post_run runs before VUnit writes the JUnit report, which an exception here would skip
            atexit.register(_exit_with_failure)

    def _take_debug_session(self, test_suite_name):
        """
        Give the debug session to the first simulation; every later one fails with the reason before yavs starts.
        Whether this simulation may run.
        """
        with self._debug_lock:
            if self._debugged is None:
                self._debugged = test_suite_name
                return True
            first = self._debugged
        flag = "--yavs-interactive" if self._interactive else "--yavs-debug"
        if self._surfer is not None and not self._interactive and self._debug_port is None:
            flag = "--yavs-surfer"
        example = "--yavs-debug=PORT" if flag == "--yavs-debug" else flag
        print(
            f"error: {flag} runs one simulation, but {test_suite_name} would start after {first}.\n"
            "Cause: the test patterns select more than one test (or one test in several configurations).\n"
            f"Fix: name exactly one test, for example 'python run.py {example} lib.tb.test_name' "
            "(see 'python run.py --list')."
        )
        return False

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
    Refuse arguments that cannot be combined with --yavs-debug
    """
    if _coverage_requested(args):
        refuse(
            "--yavs-debug and coverage cannot be combined: yavs does not collect coverage under the debugger. "
            "Drop --yavs-cov or --yavs-debug."
        )
    if getattr(args, "elaborate", False):
        refuse(
            "--yavs-debug and --elaborate cannot be combined: --elaborate only elaborates, so there is no run to debug. "
            "Drop one of them."
        )
    if getattr(args, "num_threads", 1) != 1:
        refuse(
            f"--yavs-debug runs one test, but -p {args.num_threads} asks for parallel simulations. "
            "Drop -p (or use -p 1) and name one test."
        )


def check_interactive_args(args, debug_port):
    """
    Refuse arguments that cannot be combined with --yavs-interactive
    """
    if _coverage_requested(args):
        refuse(
            "--yavs-interactive and coverage cannot be combined: yavs does not collect coverage under the debugger. "
            "Drop --yavs-cov or --yavs-interactive."
        )
    if debug_port is not None:
        refuse(
            "--yavs-interactive and --yavs-debug cannot be combined: the terminal or a debug client drives the run. "
            "Drop one of them."
        )
    if getattr(args, "elaborate", False):
        refuse(
            "--yavs-interactive and --elaborate cannot be combined: --elaborate only elaborates, so there is no run. "
            "Drop one of them."
        )
    if getattr(args, "num_threads", 1) != 1:
        refuse(
            f"--yavs-interactive runs one test, but -p {args.num_threads} asks for parallel simulations. "
            "Drop -p (or use -p 1) and name one test."
        )


def _printable(text):
    """text with what the console cannot encode replaced: a Windows code page lacks the symbols of yavs output"""
    encoding = getattr(sys.stdout, "encoding", None) or "utf-8"
    return text.encode(encoding, "replace").decode(encoding)


def _batch_key(source_file):
    """Files of one key can share a yavs analyze call: one library, standard and set of flags"""
    return (
        source_file.library.name,
        source_file.get_vhdl_standard(),
        tuple(source_file.compile_options.get("yavs.global_flags", [])),
        tuple(source_file.compile_options.get("yavs.a_flags", [])),
    )


def _library_groups(project, graph):
    """
    {library: its group}: libraries that read each other, directly or through others, share a group. Two calls at once
    into one group could each hold their own library's lock and wait for the other's (#1150).
    """
    names = {library.name.lower() for library in project.get_libraries()}
    reads = {name: set() for name in names}
    for source_file in project.get_source_files_in_order():
        mine = reads[source_file.library.name.lower()]
        mine.update(dep.library.name.lower() for dep in graph.get_direct_dependencies(source_file))
        if source_file.is_vhdl:
            text = re.sub(r"--[^\n]*", "", Path(source_file.name).read_text(encoding="latin-1"))
            for clause in LIBRARY_CLAUSE.findall(text):
                mine.update(name.strip().lower() for name in clause.split(",") if name.strip().lower() in names)
    reach = {}
    for name in names:
        seen, todo = set(), [name]
        while todo:
            for other in reads[todo.pop()] - seen:
                seen.add(other)
                todo.append(other)
        reach[name] = seen
    return {name: min({name} | {other for other in reach[name] if name in reach[other]}) for name in names}


class _Compile:
    """
    One compile in yavs analyze calls, up to `jobs` at once. A call starts at a ready file of a library group no call
    holds (yavs locks a library per call) and takes the later files of its key whose dependencies are done or in it.
    Printing and project.update stay in the main thread (docs/decisions.md "Batch and parallel compile in vunit-yavs").
    """

    def __init__(self, simif, project, printer, graph, files, groups):  # pylint: disable=too-many-arguments
        self._groups = groups  # {library: group}; a library outside is a group of its own
        self._simif = simif
        self._project = project
        self._printer = printer
        self._graph = graph
        self._order = {source_file: idx for idx, source_file in enumerate(files)}
        # Dependencies outside the list are up to date: VUnit lists everything that depends on a listed file
        self._deps = {
            source_file: graph.get_direct_dependencies(source_file) & self._order.keys() for source_file in files
        }
        self._left = list(files)  # Not started, in compile order
        self._done = set()
        self._alone = set()  # Files of a failed call, repeated one per call
        self._skipped = set()
        self._failures = []
        self._width_library = max(len(source_file.library.name) for source_file in files) + 1
        self._width_name = max(len(simplify_path(source_file.name)) for source_file in files)

    def run(self, jobs, continue_on_error):
        """Compile everything that can be compiled and return the files that failed"""
        busy = set()  # Libraries a running call holds
        stop = False
        with ThreadPoolExecutor(max_workers=jobs) as pool:
            running = {}
            while True:
                while not stop and len(running) < jobs:
                    files = self._next_call(busy)
                    if not files:
                        break
                    busy.add(self._group(files[0]))
                    running[pool.submit(self._simif._analyze, files)] = files  # pylint: disable=protected-access
                if not running:
                    break
                finished, _ = wait(running, return_when=FIRST_COMPLETED)
                for future in sorted(finished, key=lambda future: self._order[running[future][0]]):
                    files = running.pop(future)
                    busy.discard(self._group(files[0]))
                    if not self._finish(files, *future.result()) and not continue_on_error:
                        stop = True
                    sys.stdout.flush()
        if stop:
            return self._failures
        if self._left:
            raise RuntimeError(f"internal error in vunit-yavs: no call can start for {self._left[0].name}")
        for source_file in self._order:
            if source_file in self._skipped and source_file not in self._failures:
                self._status(source_file, "skipped", "rgi")
        return self._failures

    def _next_call(self, busy):
        """The files of the next call, taken out of the files left, or None when none can start now"""
        call = []
        taken = set()
        names = 0
        key = None
        for source_file in self._left:
            if not call:
                if self._group(source_file) in busy or not self._deps[source_file] <= self._done:
                    continue
                call = [source_file]
                taken = {source_file}
                if not source_file.is_vhdl or source_file in self._alone:
                    break
                key = _batch_key(source_file)
                names = len(source_file.name) + 1
                continue
            if not source_file.is_vhdl or source_file in self._alone or _batch_key(source_file) != key:
                continue
            if not all(dep in self._done or dep in taken for dep in self._deps[source_file]):
                continue
            names += len(source_file.name) + 1
            if names > MAX_BATCH_CHARS:
                break
            call.append(source_file)
            taken.add(source_file)
        if not call:
            return None
        self._left = [source_file for source_file in self._left if source_file not in taken]
        return call

    def _group(self, source_file):
        """The library group of source_file: one call at a time into it"""
        name = source_file.library.name.lower()
        return self._groups.get(name, name)

    def _finish(self, files, cmd, passed, output):
        """Report a finished call and return False when a file failed"""
        if passed:
            for source_file in files:
                self._status(source_file, "passed", "gi")
                self._project.update(source_file)
                self._done.add(source_file)
            self._printer.write(_printable(output))
            return True
        if len(files) > 1:
            # Repeated one file per call, which names the failing file; the files before it pass again
            self._alone.update(files)
            self._left = sorted(self._left + files, key=self._order.get)
            return True
        source_file = files[0]
        self._status(source_file, "failed", "ri")
        if cmd is None:
            self._printer.write(output)
        else:
            self._printer.write(_printable(f"=== Command used: ===\n{subprocess.list2cmdline(cmd)!s}\n\n"))
            self._printer.write(_printable(f"=== Command output: ===\n{output!s}\n"))
        self._failures.append(source_file)
        self._skipped.update(self._graph.get_dependent([source_file]))
        self._left = [left for left in self._left if left not in self._skipped]
        return False

    def _status(self, source_file, status, color):
        """VUnit's line for one file"""
        self._printer.write(
            f"Compiling into {(source_file.library.name + ':').ljust(self._width_library)!s} "
            f"{simplify_path(source_file.name).ljust(self._width_name)!s} "
        )
        self._printer.write(status, fg=color)
        self._printer.write("\n")


def check_surfer_args(args):
    """
    Refuse arguments that cannot be combined with --yavs-surfer
    """
    if getattr(args, "elaborate", False):
        refuse(
            "--yavs-surfer and --elaborate cannot be combined: --elaborate only elaborates, so there are no waves "
            "to show. Drop one of them."
        )
    if getattr(args, "num_threads", 1) != 1:
        refuse(
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
YAVS_MIN_VERSION = YavsVersion("0.19.1")

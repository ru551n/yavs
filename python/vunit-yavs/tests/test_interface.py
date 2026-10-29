# This Source Code Form is subject to the terms of the Mozilla Public
# License, v. 2.0. If a copy of the MPL was not distributed with this file,
# You can obtain one at http://mozilla.org/MPL/2.0/.
#
# Copyright (c) 2014-2026, Lars Asplund lars.anders.asplund@gmail.com

"""
Unit tests of the yavs interface
"""

import importlib
import io
import os
import subprocess
import tempfile
import unittest
from pathlib import Path
from types import SimpleNamespace
from unittest import mock

from vunit.exceptions import CompileError
from vunit.ostools import Process, renew_path, write_file
from vunit.project import Project
from vunit.sim_if.factory import SimulatorFactory
from vunit.sim_if.hooks import SimulatorHooks
from vunit.test.suites import encode_dict
from vunit.ui import VUnit
from vunit.vhdl_standard import VHDL

from vunit_yavs import YavsInterface, YavsVersion, check_debug_args, write_response_file
from vunit_yavs.jobserver import Jobserver

FAKE_BIN = Path(__file__).parent / "fake_bin"


def make_config(**kwargs):
    """Create a minimal test configuration."""
    config = SimpleNamespace(
        library_name="lib",
        entity_name="tb",
        architecture_name="arch",
        vhdl_configuration_name=None,
        vhdl_assert_stop_level="error",
        generics={},
        sim_options={},
    )
    for name, value in kwargs.items():
        setattr(config, name, value)
    return config


def use_fresh_factory(test):
    """Register simulators in a factory of the test's own, so each test can add the package again."""
    factory = SimulatorFactory()
    modules = ("vunit.builtins", "vunit.ui", "vunit.vunit_cli", "vunit.configuration", "vunit.source_file")
    for module in modules + ("vunit.packages",):
        if "SIMULATOR_FACTORY" not in vars(importlib.import_module(module)):
            continue  # Not every VUnit release imports the factory in every module
        patcher = mock.patch(f"{module}.SIMULATOR_FACTORY", factory)
        patcher.start()
        test.addCleanup(patcher.stop)
    return factory


class TestYavsVersion(unittest.TestCase):
    """
    Test the yavs version parsing
    """

    def test_from_version_output(self):
        version = YavsVersion.from_output("yavs 0.4.0-dev (1a2b3c4 2026-10-18)\n")
        self.assertEqual(version, YavsVersion("0.4.0"))
        self.assertEqual(str(version), "0.4.0")

    def test_versions_are_ordered(self):
        self.assertLess(YavsVersion("0.4"), YavsVersion("0.4.1"))
        self.assertLess(YavsVersion("0.9.9"), YavsVersion("0.10.0"))
        self.assertEqual(YavsVersion("1.0"), YavsVersion("1.0.0"))

    def test_unrecognized_version_warns_and_is_unknown(self):
        with self.assertWarnsRegex(RuntimeWarning, "version is unknown"):
            version = YavsVersion.from_output("no version here")
        self.assertFalse(version.is_valid)
        with self.assertRaisesRegex(ValueError, "Cannot order unknown yavs versions"):
            version < YavsVersion("1.0")  # pylint: disable=expression-not-assigned


class TestPackage(unittest.TestCase):
    """
    The package route: add_package registers the interface, its arguments and its options
    """

    def setUp(self):
        self.factory = None
        self._tempdir = tempfile.TemporaryDirectory()  # pylint: disable=consider-using-with
        self.addCleanup(self._tempdir.cleanup)
        env = mock.patch.dict("os.environ", {"VUNIT_SIMULATOR": "yavs", "VUNIT_YAVS_PATH": str(FAKE_BIN)})
        env.start()
        self.addCleanup(env.stop)

    def _vunit(self, *args):
        self.factory = use_fresh_factory(self)
        vu = VUnit.from_argv(argv=[f"--output-path={self._tempdir.name}", *args])
        vu.add_package("vunit_yavs", allow_setup=True)
        return vu

    def test_add_package_selects_yavs_with_its_options(self):
        vu = self._vunit()
        self.assertEqual(vu.get_simulator_name(), "yavs")
        self.assertIn(YavsInterface, self.factory.supported_simulators())
        self.factory.check_compile_option("yavs.a_flags", ["-Werror"])
        self.factory.check_sim_option("yavs.max_threads", 4)
        self.assertIsNone(vu._args.yavs_debug)  # pylint: disable=protected-access
        self.assertIsNone(vu._args.yavs_max_threads)  # pylint: disable=protected-access

    def test_debug_port(self):
        for argv, port in [
            (["--yavs-debug"], 4711),
            (["--yavs-debug=0"], 0),
            (["--yavs-debug=5000", "lib.tb.x"], 5000),
        ]:
            vu = self._vunit(*argv)
            vu.get_simulator_name()
            self.assertEqual(vu._args.yavs_debug, port)  # pylint: disable=protected-access

    def test_debug_followed_by_a_test_pattern_is_a_clear_error(self):
        vu = self._vunit("--yavs-debug", "lib.tb.test")
        with mock.patch("sys.stderr", new_callable=io.StringIO) as stderr, self.assertRaises(SystemExit):
            vu.get_simulator_name()
        self.assertIn("Write the port as --yavs-debug=PORT", stderr.getvalue())

    def test_max_threads_argument(self):
        vu = self._vunit("--yavs-max-threads", "3")
        vu.get_simulator_name()
        self.assertEqual(vu._args.yavs_max_threads, 3)  # pylint: disable=protected-access
        vu = self._vunit("--yavs-max-threads=0")
        with mock.patch("sys.stderr", new_callable=io.StringIO) as stderr, self.assertRaises(SystemExit):
            vu.get_simulator_name()
        self.assertIn("at least 1", stderr.getvalue())

    def test_max_threads_option_is_validated(self):
        self._vunit().get_simulator_name()
        for value in (0, -1, "2", True, 1.5):
            with self.assertRaisesRegex(ValueError, "yavs.max_threads.*at least 1"):
                self.factory.check_sim_option("yavs.max_threads", value)


class TestYavsInterface(unittest.TestCase):
    """
    Test the yavs interface
    """

    def setUp(self):
        use_fresh_factory(self).register_simulator(YavsInterface)
        self._tempdir = tempfile.TemporaryDirectory()  # pylint: disable=consider-using-with
        self.output_path = self._tempdir.name
        renew_path(self.output_path)
        self.cwd = os.getcwd()
        os.chdir(self.output_path)
        self.simif = YavsInterface(prefix="prefix", output_path=self.output_path)

    def tearDown(self):
        os.chdir(self.cwd)
        self._tempdir.cleanup()

    @property
    def std_cache_flag(self):
        return f"--std-cache={Path(self.output_path).resolve() / 'std_cache'}"

    @staticmethod
    def _project(*standards):
        """Create a project with one VHDL file per standard"""
        project = Project()
        project.add_library("lib", "lib_path")
        project.add_library("other", str(Path("libs") / "other path"))
        for idx, std in enumerate(standards):
            write_file(f"file{idx}.vhd", "")
            project.add_source_file(f"file{idx}.vhd", "lib", file_type="vhdl", vhdl_standard=VHDL.standard(std))
        return project

    def test_capabilities(self):
        self.assertTrue(YavsInterface.has_valid_exit_code())
        self.assertTrue(YavsInterface.supports_vhdl_package_generics())
        self.assertTrue(YavsInterface.supports_vhdl_call_paths())
        self.assertTrue(YavsInterface.supports_vhpi())
        self.assertFalse(YavsInterface.supports_coverage())

    @mock.patch("vunit.sim_if.check_output", autospec=True, return_value="")
    def test_compile_project(self, check_output):
        project = self._project("2019")
        self.simif.compile_project(project)
        check_output.assert_called_once_with(
            [
                str(Path("prefix") / "yavs"),
                "analyze",
                "--std=2019",
                "--work=lib:lib_path",
                "--map=lib:lib_path",
                f"--map=other:{Path('libs') / 'other path'}",
                self.std_cache_flag,
                "--color=always",
                "file0.vhd",
            ],
            env=self.simif.get_env(),
        )
        # The parent of each library is created, the library itself is left to yavs
        self.assertTrue(Path("libs").is_dir())
        self.assertFalse(Path("libs", "other path").exists())

    @mock.patch("vunit.sim_if.check_output", autospec=True, return_value="")
    def test_compile_project_extra_flags(self, check_output):
        project = self._project("2008")
        source_file = project.get_source_files_in_order()[0]
        source_file.set_compile_option("yavs.global_flags", ["--relaxed"])
        source_file.set_compile_option("yavs.a_flags", ["-Werror"])
        self.simif.compile_project(project)
        cmd = check_output.call_args[0][0]
        self.assertEqual(cmd[-3:], ["--relaxed", "-Werror", "file0.vhd"])
        self.assertIn("--std=2008", cmd)

    @mock.patch("vunit.sim_if.check_output", autospec=True, return_value="")
    def test_std_cache_is_in_the_output_path_and_not_created(self, check_output):
        self.simif = YavsInterface(prefix="prefix", output_path="rel_out")
        self.simif.compile_project(self._project("2008"))
        flags = [arg for arg in check_output.call_args[0][0] if arg.startswith("--std-cache=")]
        self.assertEqual(flags, [f"--std-cache={Path('rel_out').resolve() / 'std_cache'}"])
        path = Path(flags[0].split("=", 1)[1])
        self.assertTrue(path.is_absolute())
        self.assertEqual(path.parent, Path(self.output_path) / "rel_out")
        self.assertFalse(path.exists())

    @mock.patch("vunit.sim_if.check_output", autospec=True, return_value="")
    def test_std_cache_in_raw_flags_wins(self, check_output):
        project = self._project("2008")
        source_file = project.get_source_files_in_order()[0]
        source_file.set_compile_option("yavs.global_flags", ["--std-cache=off"])
        self.simif.compile_project(project)
        cmd = check_output.call_args[0][0]
        self.assertLess(cmd.index(self.std_cache_flag), cmd.index("--std-cache=off"))
        self.assertEqual([arg for arg in cmd if arg.startswith("--std-cache=")][-1], "--std-cache=off")

    @mock.patch("vunit.sim_if.check_output", autospec=True)
    def test_compile_failure_is_reported(self, check_output):
        check_output.side_effect = subprocess.CalledProcessError(1, "yavs", output="error[E0001]: bad")
        with self.assertRaises(CompileError):
            self.simif.compile_project(self._project("2008"))

    def test_rejects_unsupported_standards(self):
        with self.assertRaisesRegex(RuntimeError, "VHDL-2008 and VHDL-2019 only, found VHDL-2002 for file1.vhd"):
            self.simif.setup_library_mapping(self._project("2008", "2002"))

    @mock.patch("vunit.sim_if.check_output", autospec=True, return_value="")
    def test_mixed_standards_are_analyzed_with_their_own_standard(self, check_output):
        self.simif.compile_project(self._project("2008", "2019"))
        stds = {call[0][0][-1]: call[0][0][2] for call in check_output.call_args_list}
        self.assertEqual(stds, {"file0.vhd": "--std=2008", "file1.vhd": "--std=2019"})

    def _simulate(self, config, elaborate_only=False, mode_flags=(), returncode=0, suite="lib.tb", **kwargs):
        """Simulate config with a mocked yavs process and return its status, command line and process mock"""
        if kwargs:
            self.simif = YavsInterface(prefix="prefix", output_path=self.output_path, **kwargs)
        self.simif.setup_library_mapping(self._project("2008"))
        with mock.patch("vunit_yavs.Process", autospec=True) as process:
            process.NonZeroExitCode = Process.NonZeroExitCode
            if returncode:
                process.return_value.consume_output.side_effect = Process.NonZeroExitCode
            status = self.simif.simulate(self.output_path, suite, config, elaborate_only)

        # The arguments after the subcommand are passed in a response file
        cmd = process.call_args[0][0]
        response_file = Path(self.output_path) / "yavs" / "args.rsp"
        sub = "elab" if elaborate_only else "run"
        self.assertEqual(cmd, [str(Path("prefix") / "yavs"), sub] + list(mode_flags) + [f"@{response_file}"])
        args = response_file.read_bytes().decode("utf-8").split("\n")
        self.assertEqual(args.pop(), "")
        return status, cmd[:2] + args, process

    def test_simulate(self):
        runner_cfg = encode_dict({"enabled_test_cases": "a, b,,c", "output path": "C:/out path/"})
        config = make_config(
            generics={"runner_cfg": runner_cfg, "width": 8},
            sim_options={
                "yavs.global_flags": ["--threads=2"],
                "yavs.elab_flags": ["--relaxed"],
                "yavs.sim_flags": ["--exit-severity=failure", "--stop-time=1ms"],
                "disable_ieee_warnings": True,
            },
        )
        status, cmd, _ = self._simulate(config)
        self.assertTrue(status)
        self.assertEqual(
            cmd,
            [
                str(Path("prefix") / "yavs"),
                "run",
                "--work=lib:lib_path",
                "--map=lib:lib_path",
                f"--map=other:{Path('libs') / 'other path'}",
                self.std_cache_flag,
                "--color=always",
                "--ieee-warnings=off",
                "--threads=2",
                "--relaxed",
                "--exit-severity=failure",
                "--stop-time=1ms",
                "--exit-severity=error",
                "tb.arch",
                f"-grunner_cfg={runner_cfg}",
                "-gwidth=8",
            ],
        )

    def test_simulate_configuration_and_stop_level(self):
        config = make_config(vhdl_configuration_name="cfg", vhdl_assert_stop_level="failure")
        _, cmd, _ = self._simulate(config)
        self.assertEqual(cmd[-2:], ["--exit-severity=failure", "cfg"])

    def test_std_cache_on_run_and_elab_and_user_flag_wins(self):
        config = make_config(sim_options={"yavs.elab_flags": ["--std-cache=off"]})
        for elaborate_only in (False, True):
            _, cmd, _ = self._simulate(config, elaborate_only=elaborate_only)
            self.assertEqual(cmd[1], "elab" if elaborate_only else "run")
            self.assertLess(cmd.index(self.std_cache_flag), cmd.index("--std-cache=off"))
            self.assertEqual([arg for arg in cmd if arg.startswith("--std-cache=")][-1], "--std-cache=off")
        self.assertFalse((Path(self.output_path) / "std_cache").exists())

    def test_debug_keeps_the_std_cache(self):
        _, cmd, _ = self._simulate(make_config(), mode_flags=["--dap=4711"], debug_port=4711)
        self.assertIn(self.std_cache_flag, cmd)

    def test_elaborate_only(self):
        config = make_config(sim_options={"yavs.elab_flags": ["--relaxed"], "yavs.sim_flags": ["--stop-time=1ms"]})
        _, cmd, _ = self._simulate(config, elaborate_only=True)
        self.assertEqual(cmd[1], "elab")
        self.assertIn("--relaxed", cmd)
        self.assertNotIn("--stop-time=1ms", cmd)
        self.assertFalse(any(arg.startswith("--exit-severity") for arg in cmd))

    @mock.patch("vunit.sim_if.check_output", autospec=True, return_value="")
    def test_native_library_hooks_load_the_library_on_run_only(self, check_output):
        self.assertEqual(YavsInterface.foreign_language_interface, "VHPIDIRECT")
        library_file = Path("lib dir") / "libnative.so"
        registry = SimulatorHooks()
        registry.register("yavs", **YavsInterface.native_library_hooks(library_file))
        self.simif.simulator_hooks = registry
        load = f"--load={library_file}"

        self.simif.compile_project(self._project("2008"))
        self.assertNotIn(load, check_output.call_args[0][0])
        # With the run flags, after yavs.sim_flags
        _, run, _ = self._simulate(make_config(sim_options={"yavs.sim_flags": ["--stop-time=1ms"]}))
        self.assertEqual(run[run.index("--stop-time=1ms") + 1], load)
        self.assertEqual(run.count(load), 1)
        _, elab, _ = self._simulate(make_config(), elaborate_only=True)
        self.assertNotIn(load, elab)

    def test_simulate_failure_is_not_swallowed(self):
        status, _, _ = self._simulate(make_config(), returncode=1)
        self.assertFalse(status)

    def test_gui_runs_yavs_gui_with_the_same_arguments(self):
        config = make_config(
            generics={"runner_cfg": "x", "width": 8},
            sim_options={"yavs.sim_flags": ["--stop-time=1ms"], "yavs.global_flags": ["--threads=2"]},
        )
        _, plain, _ = self._simulate(config)
        _, gui, _ = self._simulate(config, mode_flags=["--gui"], gui=True)
        self.assertEqual(gui, [arg for arg in plain if arg != "--color=always"])
        self.assertFalse(any("--wave" in arg for arg in gui))

    def test_gui_elaborate_only_has_no_gui_flag(self):
        _, cmd, _ = self._simulate(make_config(), elaborate_only=True, gui=True)
        self.assertEqual(cmd[1], "elab")

    def test_max_threads_from_the_command_line(self):
        _, cmd, _ = self._simulate(make_config(), max_threads=3)
        self.assertEqual([arg for arg in cmd if arg.startswith("--threads")], ["--threads=3"])

    def test_max_threads_sim_option_overrides_the_command_line(self):
        config = make_config(sim_options={"yavs.max_threads": 5})
        _, cmd, _ = self._simulate(config, max_threads=3)
        self.assertEqual([arg for arg in cmd if arg.startswith("--threads")], ["--threads=5"])

    def test_max_threads_comes_before_raw_flags(self):
        # Last one wins in yavs, so an explicit --threads in yavs.sim_flags beats the cap
        config = make_config(sim_options={"yavs.max_threads": 5, "yavs.sim_flags": ["--threads=2"]})
        _, cmd, _ = self._simulate(config)
        self.assertEqual([arg for arg in cmd if arg.startswith("--threads")], ["--threads=5", "--threads=2"])

    def test_no_max_threads_passes_no_thread_count(self):
        _, cmd, _ = self._simulate(make_config())
        self.assertFalse(any(arg.startswith("--threads") for arg in cmd))

    def test_max_threads_not_for_elaborate_only(self):
        _, cmd, _ = self._simulate(make_config(sim_options={"yavs.max_threads": 5}), elaborate_only=True)
        self.assertFalse(any(arg.startswith("--threads") for arg in cmd))

    def test_debug_runs_yavs_under_dap_with_the_same_arguments(self):
        config = make_config(generics={"runner_cfg": "x"}, sim_options={"yavs.sim_flags": ["--stop-time=1ms"]})
        _, plain, _ = self._simulate(config)
        _, debug, _ = self._simulate(config, mode_flags=["--dap=4711"], debug_port=4711)
        self.assertEqual(debug, plain)

    def test_debug_drops_the_thread_cap(self):
        config = make_config(sim_options={"yavs.max_threads": 5})
        _, cmd, _ = self._simulate(config, mode_flags=["--dap=0"], debug_port=0, max_threads=3)
        self.assertFalse(any(arg.startswith("--threads") for arg in cmd))

    def test_debug_does_not_apply_to_elaborate_only(self):
        _, cmd, _ = self._simulate(make_config(), elaborate_only=True, debug_port=4711)
        self.assertEqual(cmd[1], "elab")

    def test_debug_refuses_a_second_simulation_before_starting_yavs(self):
        self._simulate(make_config(), mode_flags=["--dap=4711"], debug_port=4711, suite="lib.tb.first")
        with (
            mock.patch("vunit_yavs.Process", autospec=True) as process,
            self.assertRaisesRegex(
                RuntimeError, r"--yavs-debug runs one simulation, but lib.tb.second would start after lib.tb.first"
            ) as ctx,
        ):
            self.simif.simulate(self.output_path, "lib.tb.second", make_config(), False)
        process.assert_not_called()
        self.assertIn("python run.py --list", str(ctx.exception))

    def test_debug_shows_where_to_attach(self):
        callback = YavsInterface._debug_output("lib.tb.t")  # pylint: disable=protected-access
        with mock.patch("sys.__stdout__", new_callable=io.StringIO) as terminal, mock.patch("builtins.print"):
            callback("yavs: DAP listening on 127.0.0.1:40123")
            callback("some other line")
        self.assertEqual(terminal.getvalue(), "Debugging lib.tb.t: attach a DAP client to 127.0.0.1:40123\n")

    def test_debug_rejects_conflicting_arguments(self):
        base = {"gui": False, "elaborate": False, "num_threads": 1}
        check_debug_args(SimpleNamespace(**base))
        for change, message in [
            ({"gui": True}, "--yavs-debug and --gui cannot be combined"),
            ({"elaborate": True}, "--yavs-debug and --elaborate cannot be combined"),
            ({"num_threads": 4}, "-p 4 asks for parallel simulations"),
        ]:
            with self.assertRaisesRegex(RuntimeError, message):
                check_debug_args(SimpleNamespace(**dict(base, **change)))

    def test_response_file_keeps_arguments_verbatim(self):
        args = [
            "-grunner_cfg=" + encode_dict({"enabled_test_cases": 'te"st, x', "output path": "C:\\out path\\"}),
            "@not_a_response_file",
            "",
            "  spaces  ",
            "-gtext=\u00e5\u00e4\u00f6",
        ]
        file_name = write_response_file(Path(self.output_path) / "args.rsp", args)
        self.assertEqual(Path(file_name).read_bytes().decode("utf-8"), "".join(arg + "\n" for arg in args))

    def test_response_file_rejects_line_breaks(self):
        for arg in ["-gx=a\nb", "-gx=a\r"]:
            with self.assertRaisesRegex(ValueError, "cannot contain line breaks"):
                write_response_file(Path(self.output_path) / "args.rsp", [arg])

    def test_check_version(self):
        with mock.patch.object(YavsInterface, "_get_version_output", return_value="yavs 0.10.0\n"):
            self.assertEqual(YavsInterface.check_version("prefix"), YavsVersion("0.10.0"))

        with (
            mock.patch.object(YavsInterface, "_get_version_output", return_value="garbage"),
            self.assertWarns(RuntimeWarning),
            self.assertRaisesRegex(RuntimeError, "Cannot parse the yavs version from 'garbage'"),
        ):
            YavsInterface.check_version("prefix")

        with (
            mock.patch.object(YavsInterface, "_get_version_output", side_effect=OSError("boom")),
            self.assertRaisesRegex(RuntimeError, "--version' failed: boom"),
        ):
            YavsInterface.check_version("prefix")

        with (
            mock.patch("vunit_yavs.YAVS_MIN_VERSION", YavsVersion("1.0")),
            mock.patch.object(YavsInterface, "_get_version_output", return_value="yavs 0.10.0"),
            self.assertRaisesRegex(
                RuntimeError,
                r"yavs 0.10.0 at prefix.yavs is too old for vunit-yavs, which needs yavs 1.0.0 or later\. Install",
            ),
        ):
            YavsInterface.check_version("prefix")

    def test_from_args_without_executable(self):
        args = SimpleNamespace(gui=False)
        with (
            mock.patch.object(YavsInterface, "find_prefix", return_value=None),
            self.assertRaisesRegex(FileNotFoundError, "YAVS executable not found"),
        ):
            YavsInterface.from_args(args, self.output_path)

    def test_from_args_with_a_namespace_of_the_run_script(self):
        # A namespace the run script built itself has no package arguments
        with mock.patch.dict("os.environ", {"VUNIT_YAVS_PATH": str(FAKE_BIN)}):
            simif = YavsInterface.from_args(SimpleNamespace(gui=False), self.output_path)
        self.assertIsNone(simif._debug_port)  # pylint: disable=protected-access
        self.assertIsNone(simif._max_threads)  # pylint: disable=protected-access

    def _from_args(self, **kwargs):
        with mock.patch.dict("os.environ", {"VUNIT_YAVS_PATH": str(FAKE_BIN)}):
            return YavsInterface.from_args(SimpleNamespace(gui=False, **kwargs), self.output_path)

    def test_jobserver_only_with_the_flag_and_parallel_runs(self):
        self.assertIsNone(self._from_args(num_threads=4)._jobserver)  # pylint: disable=protected-access
        self.assertIsNone(self._from_args(num_threads=1, yavs_jobserver=True)._jobserver)
        jobserver = self._from_args(num_threads=4, yavs_jobserver=True)._jobserver
        self.assertEqual(jobserver.num_threads, 4)
        jobserver.close()

    def test_jobserver_on_windows_is_a_clear_error(self):
        with (
            mock.patch("vunit_yavs.sys.platform", "win32"),
            self.assertRaisesRegex(RuntimeError, "--yavs-jobserver needs POSIX named pipes"),
        ):
            self._from_args(num_threads=4, yavs_jobserver=True)

    def test_simulate_with_jobserver(self):
        jobserver = Jobserver(4, interval=3600)
        self.addCleanup(jobserver.close)
        config = make_config(sim_options={"yavs.max_threads": 2})
        _, _, process = self._simulate(config, jobserver=jobserver)
        flags = process.call_args[1]["env"]["MAKEFLAGS"]
        self.assertRegex(flags, r"^-j4 --jobserver-auth=fifo:\S+/fifo0$")
        self.assertFalse(Path(flags.split("fifo:")[1]).exists())  # Removed when the test ended
        self.assertEqual(jobserver._clients, [])  # pylint: disable=protected-access

    def test_simulate_with_jobserver_unregisters_on_error(self):
        jobserver = Jobserver(4, interval=3600)
        self.addCleanup(jobserver.close)
        self.simif = YavsInterface(prefix="prefix", output_path=self.output_path, jobserver=jobserver)
        self.simif.setup_library_mapping(self._project("2008"))
        with mock.patch("vunit_yavs.Process", autospec=True) as process, self.assertRaises(OSError):
            process.NonZeroExitCode = Process.NonZeroExitCode
            process.side_effect = OSError("boom")
            self.simif.simulate(self.output_path, "lib.tb", make_config(), False)
        self.assertEqual(jobserver._clients, [])  # pylint: disable=protected-access

    def test_from_args_checks_debug_before_the_executable(self):
        args = SimpleNamespace(gui=True, yavs_debug=4711)
        with (
            mock.patch.object(YavsInterface, "find_prefix", side_effect=AssertionError("not reached")),
            self.assertRaisesRegex(RuntimeError, "--yavs-debug and --gui"),
        ):
            YavsInterface.from_args(args, self.output_path)

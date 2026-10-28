"""cocotb Python runner for yavs. tools/cocotb/run.py copies it into the venv and appends its
registration to cocotb_tools/runner.py (SUPPORTED_RUNNERS, cocotb's extension point), so nothing is
imported at interpreter start."""

import os
import shutil
from pathlib import Path

import cocotb_tools.config
from cocotb_tools import runner as r


def vhpi_lib() -> str:
    ext = ".dll" if os.name == "nt" else ".so"
    return str(Path(cocotb_tools.config.libs_dir) / f"libcocotbvhpi_yavs{ext}")


class Yavs(r.Runner):
    supported_gpi_interfaces = {"vhdl": ["vhpi"]}

    def _simulator_in_path(self) -> None:
        if shutil.which(self._yavs()) is None:
            raise SystemExit(f"ERROR: yavs executable not found: {self._yavs()} (set YAVS_BIN or PATH)")

    def _yavs(self) -> str:
        return os.environ.get("YAVS_BIN", "yavs")

    def _get_include_options(self, includes):
        raise RuntimeError("yavs is VHDL only: no include directories")

    def _get_define_options(self, defines):
        raise RuntimeError("yavs is VHDL only: no defines")

    def _get_parameter_options(self, parameters):
        return [f"-g{name}={value}" for name, value in parameters.items()]

    def _use_external_viewer(self) -> bool:
        return True

    def _waves_file(self):
        return f"{self.sim_hdl_toplevel}.fst"

    def _maps(self):
        build = r.get_abs_path(self.build_dir)
        return [f"--map={d.name}:{d}" for d in sorted(build.iterdir()) if (d / "_YAVS_LIB").exists()]

    def _lib(self, name):
        return f"{name}:{r.get_abs_path(self.build_dir) / name}"

    def _build_command(self):
        for s in self._sources + self._vhdl_sources:
            if s.tag is not r.VHDL:
                raise ValueError(f"yavs only supports VHDL. {str(s.value)!r} cannot be compiled.")
        (r.get_abs_path(self.build_dir) / self.hdl_library).mkdir(parents=True, exist_ok=True)
        args = [a.value for a in self._build_args if a.tag in (r.VHDL, None)]
        srcs = [str(s.value) for s in self._sources + self._vhdl_sources]
        # every library already in the build directory (earlier build() calls) is mapped
        return [[self._yavs(), "analyze", "--std=2008", f"--work={self._lib(self.hdl_library)}", *self._maps(), *args, *srcs]]

    def _test_command(self):
        res = []
        if self.timescale:
            unit = self.timescale[1].lstrip("1")
            res = [f"--resolution={'sec' if unit == 's' else unit}"]
        return [
            [
                *self._get_sim_cmd_prefix(),
                self._yavs(),
                "run",
                "--std=2008",
                f"--work={self._lib(self.hdl_toplevel_library)}",
                *self._maps(),
                *res,
                *self.elab_args,
                *self._get_parameter_options(self.parameters),
                *self.test_args,
                f"--vhpi={vhpi_lib()}",
                *([f"--wave={self._waves_file()}"] if self.waves or self.gui else []),
                self.sim_hdl_toplevel,
                *self.plusargs,
                *self._get_sim_cmd_suffix(),
            ]
        ]

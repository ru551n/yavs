# This Source Code Form is subject to the terms of the Mozilla Public
# License, v. 2.0. If a copy of the MPL was not distributed with this file,
# You can obtain one at http://mozilla.org/MPL/2.0/.
#
# Copyright (c) 2014-2026, Lars Asplund lars.anders.asplund@gmail.com

"""
Setup of the native_demo VUnit package: builds native.c with the C compiler of the host and
registers the hooks that make yavs load it. A package for another simulator registers its own flags
in the same place; the yavs hooks come from the simulator interface (`native_library_hooks`).
"""

import os
import shutil
import subprocess
import sys
import sysconfig
from pathlib import Path

from vunit_yavs import YavsInterface

SOURCE = Path(__file__).parent / "native_demo" / "native.c"


def compiler():
    """The C compiler to use: CC, else the first of gcc, cc and clang on PATH, else None"""
    return os.environ.get("CC") or next((c for c in ("gcc", "cc", "clang") if shutil.which(c)), None)


def python_flags():
    """Compiler flags that embed this Python, or None when its headers or library are missing"""
    include = Path(sysconfig.get_paths()["include"])
    libdir = sysconfig.get_config_var("LIBDIR")
    lib = sysconfig.get_config_var("LDLIBRARY")
    if sys.platform != "linux" or not (include / "Python.h").is_file() or not libdir or not lib:
        return None
    if not (Path(libdir) / lib).is_file() or not lib.endswith(".so"):
        return None
    name = f"python{sysconfig.get_config_var('LDVERSION')}"
    return ["-DWITH_PYTHON", f"-I{include}", f"-L{libdir}", f"-l{name}", f"-Wl,-rpath,{libdir}"]


def available():
    """What the host can build: the native library and, on top of it, Python from VHDL"""
    return {"native": compiler() is not None, "python": compiler() is not None and python_flags() is not None}


def setup(context):
    """Build the library and register the hooks that load it into every yavs simulation"""
    build = Path(context.output_path) / "native_demo"
    build.mkdir(parents=True, exist_ok=True)
    library = build / ("native_demo.dll" if sys.platform == "win32" else "libnative_demo.so")
    flags = (python_flags() or []) if os.environ.get("NATIVE_DEMO_PYTHON") == "1" else []
    command = [compiler(), "-shared", "-Wall", "-Werror", str(SOURCE), "-o", str(library), *flags]
    if sys.platform != "win32":
        command.insert(2, "-fPIC")
    subprocess.run(command, check=True, cwd=build)

    hooks = YavsInterface.native_library_hooks(library)
    # An embedded interpreter finds its standard library from the installation it was built for
    hooks["run_env"] = lambda simulator_interface, env: dict(env, PYTHONHOME=sys.base_prefix)
    context.register_simulator_hooks("yavs", **hooks)

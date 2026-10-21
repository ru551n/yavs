"""Local configuration of the reference simulators (untracked tools/sims.local.conf).

The tracked tools know the simulators only by label (sim1, sim2, ...). Everything that names a
product (executables, command templates, output markers) lives in the untracked file
tools/sims.local.conf, an INI file with one section per label:

    [sim1]
    discrep = <analyse bash>|||<elaborate-and-run bash>[|||<crash regex>]
    ...

A key can be overridden by the environment variable SIMCONF_<KEY>_<LABEL> (upper case).
Command line: `python3 tools/simconf.py LABEL KEY [DEFAULT]` prints the value (for shell scripts).
Keys are documented where they are used (tools/AGENTS.md lists them).
"""
import configparser
import os
import sys
from pathlib import Path

CONF = Path(os.environ.get("YAVS_SIMS_CONF") or Path(__file__).resolve().parent / "sims.local.conf")


def labels():
    cp = _read()
    return cp.sections()


def _read():
    cp = configparser.RawConfigParser(delimiters=("=",), interpolation=None)
    cp.optionxform = str
    if CONF.exists():
        cp.read(CONF)
    return cp


def get(label, key, default=None):
    v = os.environ.get(f"SIMCONF_{key}_{label}".upper())
    if v is not None:
        return v
    cp = _read()
    return cp.get(label, key, fallback=default) if cp.has_section(label) else default


if __name__ == "__main__":
    v = get(sys.argv[1], sys.argv[2], sys.argv[3] if len(sys.argv) > 3 else None)
    if v is None:
        sys.exit(f"{sys.argv[1]}: key {sys.argv[2]} not configured (tools/sims.local.conf)")
    print(v)

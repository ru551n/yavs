Test runner recipes
===================

This page shows how to drive yavs from an external Python test runner or from a plain script,
and how to compile the common verification libraries and vendor libraries with it. yavs has no
installer command and no precompiled libraries: every library is built by analyzing VHDL sources
into a named library directory, and the commands below are all a test runner needs to issue.

The commands
------------

A test runner only needs three commands. It builds the argument list itself, without a shell, so
file names with spaces work.

* ``yavs analyze --std=STD --work=NAME:PATH FILE...`` analyzes files into a library.
* ``yavs elab ... TOP`` elaborates a top level and stops (a check that the design builds).
* ``yavs run ... TOP`` elaborates and simulates.

Options use the ``--name=value`` form, in any order before the positional arguments; if an option
is repeated, the last one wins. Generics are ``-gNAME=VALUE``, one argument each. A string
generic value is taken verbatim, without VHDL quotes, so a long value with spaces, colons or
backslashes needs no escaping.

.. code-block:: python

    import subprocess

    def run(*args):
        return subprocess.run(["yavs", *args], capture_output=True, text=True)

    assert run("--version").returncode == 0
    assert run("analyze", "--std=2008", "--work=lib:out/lib", "pkg.vhd", "tb.vhd").returncode == 0
    result = run("run", "--work=lib:out/lib", "--map=lib:out/lib", "-gWIDTH=8", "tb.arch")
    print(result.stdout, result.stderr)

``yavs --version`` prints ``yavs MAJOR.MINOR.PATCH[-PRERELEASE] [(details)]`` on the first line
of standard output. A runner can parse the first ``MAJOR.MINOR`` it finds to refuse a version that
is too old. The executable is looked up on ``PATH``.

Response files
--------------

An argument ``@FILE`` is replaced in place by the arguments read from FILE, for every
subcommand. The file is UTF-8, with one argument per line, taken verbatim: no quoting, no escapes,
no comments. Expansion is not recursive. A missing file is a usage error (exit 2). Use this for
long generic values (the Windows command line is limited to 32767 characters) and for long file
lists:

.. code-block:: bash

    printf '%s\n' --work=lib:out/lib --exit-severity=error tb.arch '-grunner_cfg=active python runner : true' > args.rsp
    yavs run @args.rsp

Libraries
---------

A library is a directory. ``--work=NAME:PATH`` is the library being analyzed into (or the one
holding the top level), and ``--map=NAME:PATH`` maps every other library. Mapping the work library
as well is allowed. Names are case-insensitive; the path is split at the first colon, so it may
contain spaces and a Windows drive letter. A missing directory is created by ``analyze``.
``std`` and ``ieee`` are built into the executable and cannot be mapped.

``run`` and ``elab`` never write to a library, so any number of simulations can run at once on the
same libraries. Several ``analyze`` calls into one library at the same time are also safe: the
library is locked while its manifest is updated.

Standard per file
-----------------

``--std=2008`` or ``--std=2019`` applies to the files of one ``analyze`` call (default ``2008``),
so a project that mixes both standards calls ``analyze`` once per standard. yavs records the
standard of each file. ``run`` and ``elab`` re-analyze every file with its recorded standard, and
need no ``--std``. The whole design uses one revision of ``std`` and ``ieee``, the newest standard
among its files; a construct that is not legal under that revision fails early with ``E1007``,
and one whose meaning changes is warning ``W1008``.

Simulation options
------------------

* ``--exit-severity=note|warning|error|failure``: an assertion or report at or above the level
  stops the simulation and gives exit status 1. The default is ``failure``.
* ``--threads=N``: evaluate processes on N threads. The default is 1, and the output is identical
  for any N. A test runner that already runs simulations in parallel should leave it at 1.
* ``--color=always`` keeps colour on a pipe, ``--ieee-warnings=off`` silences warnings raised
  inside ``ieee`` packages, ``--wave=FILE.fst`` records the design for a waveform viewer.
* ``--message-format=json`` prints diagnostics as one JSON object per line on standard error.

Exit codes
----------

.. list-table::
   :header-rows: 1

   * - Code
     - Meaning
   * - 0
     - Success. For ``run``: the simulation ended without failure (no more events, ``stop`` or
       ``finish`` with status 0, or ``--stop-time`` reached).
   * - 1
     - ``analyze``: a file has errors. ``run``: a failed assertion at or above the exit severity,
       an ``error`` or ``failure`` report at any point, ``stop`` or ``finish`` with a non-zero
       status, or a runtime error.
   * - 2
     - Usage, library or elaboration error.
   * - 3
     - Internal error. Please report it.

A runner should never ignore a non-zero status: yavs stops at the first failing stage and does not
exit 0 after printing an error. Diagnostics go to standard error with a stable code;
``yavs --explain CODE`` prints the long text of one.

Libraries from source
---------------------

Because there is no precompiled form, adding a library is analyzing its sources, in dependency
order, into a library of your choice. A verification library that expects to be called
``mylib`` is analyzed with ``--work=mylib:PATH``, and a design that uses it maps it:

.. code-block:: bash

    yavs analyze --std=2008 --work=mylib:out/mylib src/pkg_a.vhd src/pkg_b.vhd
    yavs analyze --std=2008 --work=work:out/work --map=mylib:out/mylib tb.vhd
    yavs run --work=work:out/work --map=mylib:out/mylib tb

Analysis is on demand: ``elab`` and ``run`` analyze only the units the top level reaches, so a
large library does not slow down every run. A test runner that compiles a project this way
needs one ``analyze`` call per library and standard, with its files in compile order (or one
``@filelist`` response file with a file per line), and then passes ``--map`` for every library
to ``run``.

Vendor libraries
----------------

Vendor simulation libraries are not shipped, because their sources are licensed by the vendor.
The pattern is to read the sources from a local installation, found through an environment
variable, and analyze them into a named library once:

.. code-block:: python

    import os, pathlib, subprocess

    root = os.environ.get("VENDOR_SIM_DIR")      # set by the user, e.g. the install's simulation folder
    if root is None:
        raise SystemExit("set VENDOR_SIM_DIR to the vendor simulation source directory")
    files = sorted(str(p) for p in pathlib.Path(root).glob("*.vhd"))
    subprocess.run(["yavs", "analyze", "--std=2008", "--work=vendorlib:out/vendorlib", *files],
                   check=True)

Replace the file selection with the compile order the vendor documents. Vendor models are
supported for functional simulation only: timing models run with zero delay and no SDF
annotation is read. If a file is rejected, run ``yavs analyze`` on it alone to see the diagnostic.

For the ``unisim`` and ``unimacro`` libraries, analyze the vendor's component package
(``vcomponents``) into ``unisim`` before the designs that use it; the vendor's own analyze-order
lists give the order for the primitive models. A design that only instantiates primitives needs
the component package and nothing else for analysis:

.. code-block:: bash

    yavs analyze --std=2008 --work=unisim:out/unisim "$VENDOR_SIM_DIR/unisims/unisim_retarget_VCOMP.vhd"
    yavs analyze --std=2008 --work=work:out/work --map=unisim:out/unisim my_design.vhd

The primitive models themselves use the VITAL and Synopsys packages in ``ieee``, which yavs
ships built in.
``tools/corpus/run.py unisim-2008`` measures this against a local installation when
``YAVS_VENDOR_UNISIM`` is set and is skipped otherwise.

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

Options use the ``--name=value`` form, in any order before the positional arguments (yavs also
accepts ``--name value`` for people typing by hand); if an option that takes one value is repeated,
the last one wins. Generics are ``-gNAME=VALUE``, one argument each, before or after the top
level, and ``NAME`` is case-insensitive. A string generic value is taken verbatim, without VHDL
quotes, so a long value with spaces, colons or backslashes needs no escaping. A ``-g`` for a generic
the top level does not have is warning ``W2105``, not an error. After the top level of ``run``, an
argument ``+NAME`` or ``+NAME=VALUE`` is a plusarg for VHPI plugins; any other extra argument is a
usage error.

The examples on this page use the testbench ``docs/examples/hello_tb.vhd`` of the source tree
(shown in full in :doc:`usage`), which has the generics ``GREETING`` and ``EXPECTED``.

.. code-block:: python

    import subprocess

    def run(*args):
        return subprocess.run(["yavs", *args], capture_output=True, text=True)

    assert run("--version").returncode == 0
    assert run("analyze", "--std=2008", "--work=lib:out/lib", "hello_tb.vhd").returncode == 0
    result = run("run", "--work=lib:out/lib", "--map=lib:out/lib", "-gEXPECTED=3", "hello_tb.sim")
    print(result.returncode)    # 0
    print(result.stdout)        # the reports of the simulation
    print(result.stderr)        # diagnostics: empty here

``yavs --version`` prints ``yavs MAJOR.MINOR.PATCH[-PRERELEASE] [(details)]`` on the first line
of standard output. A runner can parse the first ``MAJOR.MINOR`` it finds to refuse a version that
is too old. The details say whether a compiled tier is built in (``compiled tier: llvm``,
``compiled tier: cranelift``, or ``interpreter only, no compiled tier``). The executable is looked
up on ``PATH``. The reports of a simulation (``report``, ``assert``, ``std.textio`` output to
``output``) go to standard output; every diagnostic of yavs itself goes to standard error.

Response files
--------------

An argument ``@FILE`` is replaced in place by the arguments read from FILE, for every
subcommand. The file is UTF-8, with one argument per line, taken verbatim: no quoting, no escapes,
no comments. Expansion is not recursive. A missing file is a usage error (exit 2). Use this for
long generic values (the Windows command line is limited to 32767 characters) and for long file
lists:

.. code-block:: bash

    printf '%s\n' --work=lib:out/lib --exit-severity=error hello_tb.sim '-gGREETING=hi there: C:\x, 100%' > args.rsp
    yavs run @args.rsp

The first report of that run is ``** Note: 32000000fs+0: hi there: C:\x, 100%, count is 3``.

Libraries
---------

A library is a directory. ``--work=NAME:PATH`` is the library being analyzed into (or the one
holding the top level), and ``--map=NAME:PATH`` maps every other library. Mapping the work library
as well is allowed. Names are case-insensitive; the path is split at the first colon, so it may
contain spaces and a Windows drive letter. A missing directory is created by ``analyze``, with its
parents. A mapped library that does not exist, or is empty, is an empty library. The work library
of ``run`` and ``elab`` is the exception: a directory with no index, because nothing was analyzed
into it, is error ``L0001`` (exit 2). ``std`` and ``ieee`` are built into the executable and cannot be
mapped (error ``E0102``, exit 2).

The library records the absolute path of each source file and its content hash, and ``run`` and
``elab`` read the sources again. If a file moved, ``run`` stops with ``L0033``; if it changed since
it was analyzed, with ``L0034``: analyze it again.

``run`` and ``elab`` never write to a library, so any number of simulations can run at once on the
same libraries. Several ``analyze`` calls into one library at the same time are also safe: the
library is locked while its manifest is updated.

Standard per file
-----------------

``--std=93``, ``--std=2008`` or ``--std=2019`` applies to the files of one ``analyze`` call (default
``2008``), so a project that mixes standards calls ``analyze`` once per standard. yavs records the
standard of each file. ``run`` and ``elab`` re-analyze every file with its recorded standard, and
need no ``--std``. The whole design uses one revision of ``std`` and ``ieee``, the newest standard
among its files; a construct that is not legal under that revision fails early with ``E1007``,
and one whose meaning changes is warning ``W1008``.

``--std=93`` is VHDL-1993: ``std_logic_vector`` is a type of its own (so a package may overload a
subprogram on both ``std_logic_vector`` and ``std_ulogic_vector``), the 1993 ``numeric_std``,
``std_logic_textio`` and ``std.textio`` are used, a shared variable can have any type, and the
words 2008 reserved (``default``, ``context``, ``force``, ...) and its predefined names
(``to_string``, ``maximum``, ``integer_vector``, ...) are free. A design made only of 93 files runs
against these packages. A 93 file in a design with 2008 files is checked as 93 but sees the 2008
packages.

Simulation options
------------------

* ``--exit-severity=note|warning|error|failure``: an assertion or report at or above the level
  stops the simulation and gives exit status 1. Without the option the level is ``failure``, an
  ``error`` report is printed, the run continues, and the exit status is still 1 at the end. With the
  option given, even as ``failure``, only reports at or above the level affect the exit status.
* ``--stop-time=TIME`` (``10ms``, ``10 ns``) ends the run at that time with exit status 0.
* ``--threads=N``: evaluate processes on N threads. The default is 1, and the output is identical
  for any N. A test runner that already runs simulations in parallel should leave it at 1.
* ``--color=always`` keeps colour on a pipe (``--color=never``, or ``NO_COLOR`` set, turns it off),
  ``--wave=FILE.fst`` records the design for a waveform viewer. ``--ieee-warnings=on|off`` is
  accepted, but it has no effect yet: warnings raised inside ``ieee`` packages are always printed.
* ``--message-format=human|short|json``: ``short`` prints one line per diagnostic
  (``FILE:LINE:COLUMN: error[CODE]: message``), ``json`` one JSON object per line on standard
  error. In ``json`` mode, when ``analyze`` stops at a failing file of several, plain text lines
  that name the failing file and the files analyzed before it follow the JSON objects.
* ``-Werror`` makes every warning an error, ``-Werror=CODE``, ``-Wno-CODE`` and ``-WCODE`` change one
  code. ``analyze`` then exits 1, ``run`` and ``elab`` exit 2.
* ``--encoding=auto|latin1|utf8`` says how source files are read (UTF-8 when valid and ISO-8859-1
  otherwise, by default).
* ``--no-cache`` (``analyze``, ``elab``, ``run``) skips the compilation cache; ``yavs cache clean
  --work=NAME:PATH`` empties it (:doc:`usage`).

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
     - ``analyze``: a file has errors. ``run``: a failed assertion at or above the exit severity
       (without ``--exit-severity``: an ``error`` or ``failure`` report at any point), or a
       runtime error.
   * - 2
     - Usage, library or elaboration error, including a runtime error while the design is being
       built, before time 0.
   * - 3
     - Internal error. Please report it.
   * - 1 to 125
     - ``run``: ``stop(N)`` or ``finish(N)`` with a non-zero ``N`` gives the status ``N`` when it is
       in 1 to 125 and 1 otherwise. So ``finish(2)`` and ``finish(3)`` exit with 2 and 3, the same
       numbers as a usage and an internal error: read the diagnostics on standard error.
   * - 130
     - ``run``: stopped by Ctrl-C or SIGTERM (with ``--wave`` the first Ctrl-C gives 1).
   * - 141
     - ``run``: a write error on standard output, such as a closed pipe.

A runner should never ignore a non-zero status: yavs stops at the first failing stage and does not
exit 0 after printing an error. Diagnostics go to standard error with a stable code;
``yavs --explain CODE`` prints the long text of one on standard output (exit 2 for an unknown
code).

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
lists (``vhdl_analyze_order``) give the order for the primitive models. A design that only
instantiates primitives needs the component package and nothing else for analysis. To simulate it,
also analyze the primitives it instantiates: without them the instance stays unbound (warning
``W2108``) and its outputs stay ``'U'``. With ``VENDOR_SIM_DIR`` naming the directory that holds
``unisims/``:

.. code-block:: bash

    U="$VENDOR_SIM_DIR/unisims"
    yavs analyze --std=2008 --work=unisim:out/unisim \
        "$U/unisim_VPKG.vhd" "$U/unisim_retarget_VCOMP.vhd" "$U/primitive/BUFG.vhd"
    yavs analyze --std=2008 --work=work:out/work --map=unisim:out/unisim my_design.vhd
    yavs run --work=work:out/work --map=unisim:out/unisim my_design

The primitive models themselves use the VITAL and Synopsys packages in ``ieee``, which yavs
ships built in.
``tools/corpus/run.py unisim-2008`` measures this against a local installation when
``YAVS_VENDOR_UNISIM`` is set and is skipped otherwise.

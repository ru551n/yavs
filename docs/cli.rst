Command-line reference
======================

``yavs --help`` and ``yavs <command> --help`` list every option. Write options as
``--name=value``; they go before or after the positional arguments.

Commands
--------

.. list-table::
   :header-rows: 1
   :widths: 45 55

   * - Command
     - What it does
   * - ``yavs analyze [options] FILE...``
     - Check files and store them in the ``--work`` library, in order; the first failing file
       stops.
   * - ``yavs run [options] TOP [-gNAME=VALUE...] [+ARG...]``
     - Elaborate and simulate ``TOP``: an entity, ``ENTITY.ARCH`` or a configuration.
   * - ``yavs elab [options] TOP``
     - Elaborate only: check that the design builds.
   * - ``yavs cov merge|report|rank ...``
     - Merge and report coverage databases (:doc:`coverage`).
   * - ``yavs cache clean --work=NAME:PATH``
     - Empty the compilation cache of a library (:doc:`running-faster`).
   * - ``yavs parse FILE...``
     - Only check that files parse.
   * - ``yavs --explain CODE``
     - Explain a diagnostic code.
   * - ``yavs --version``
     - Print the version and whether a compiled tier is built in.

Libraries and sources
---------------------

``analyze``, ``elab`` and ``run``:

* ``--work=NAME:PATH``: the library to analyze into, or that holds the top level.
* ``--map=NAME:PATH``: another library (repeatable). ``std`` and ``ieee`` are built in.
* ``--std=93|2008|2019``: the standard of the files of this ``analyze`` call (default ``2008``).
  ``run`` uses the standard recorded per file.
* ``--relaxed``: a few errors common in verification code become warnings.
* ``--psl``: read PSL in ``-- psl`` comments.
* ``-D NAME=VALUE``: define a conditional analysis identifier. ``run`` and ``elab`` read the
  sources again, so pass the same definitions to them as to ``analyze``.
* ``--encoding=auto|latin1|utf8``: how source files are read (default: UTF-8 when valid, else
  Latin-1).
* ``--no-cache``: do not use the compilation cache.

Simulation
----------

``run``:

* ``-gNAME=VALUE``: set a generic of the top level. A string value is taken as written, without
  VHDL quotes.
* ``--stop-time=TIME``: end the run at that time (``10ms``, ``10 ns``) with exit status 0.
* ``--exit-severity=note|warning|error|failure``: a report at or above this level stops the run
  with exit status 1 (default ``failure``; an ``error`` report then still makes the exit status 1
  at the end).
* ``--resolution=fs|ps|ns|us|ms|sec``: the time resolution limit (default ``fs``).
* ``--max-deltas=N``: delta cycles allowed at one time before error ``E3002`` (default 10000).
* ``--hang-report``: when a run ends without ``finish`` or ``stop``, say what the waiting processes
  wait for.
* ``--shuffle[=SEED]``: run processes in a random order, to find testbenches that depend on the
  order; the seed is printed so the run can be repeated.
* ``+NAME[=VALUE]`` after the top level: arguments for VHPI plugins.

Output and diagnostics
----------------------

All commands:

* ``--message-format=human|short|json``: the form of diagnostics on standard error.
* ``--color=auto|always|never``: colour of diagnostics (``NO_COLOR`` is honoured).
* ``-Werror``, ``-Werror=CODE``, ``-Wno-CODE``, ``-WCODE``: turn warnings into errors, or change
  one code.

Waveforms, coverage, debugging, plugins
---------------------------------------

``run``:

* ``--wave=FILE.fst``, ``--wave-include=GLOB``, ``--wave-exclude=GLOB``, ``--wave-variables``,
  ``--wave-max-array=N``: :doc:`waveforms`.
* ``--cov[=FILE]``, ``--cov-kind=LIST``, ``--cov-test=NAME``, ``--cov-toggle-max=N``,
  ``--cov-exclude-translate-off``: :doc:`coverage`.
* ``--dap=PORT|stdio``: :doc:`debugging`.
* ``--vhpi=LIB[:ENTRY]``, ``--load=LIB``: :doc:`vhpi`.

Performance
-----------

``run``:

* ``--tier=auto|clift|interp``: the execution tier (:doc:`running-faster`).
* ``--threads=N``: evaluate processes on N threads (at least 1, default 1); under a token pool in ``MAKEFLAGS``
  the extra threads run only on tokens taken from it.
* ``--time-report``, ``--profile[=FILE]``: where the time goes.
* ``--std-cache=DIR|off``: where the analyzed ``std`` and ``ieee`` packages are kept.

Exit status
-----------

.. list-table::
   :header-rows: 1
   :widths: 15 85

   * - Status
     - Meaning
   * - 0
     - Success. For ``run``: the simulation ended without failure (no more events, ``stop`` or
       ``finish`` with 0, or ``--stop-time`` reached).
   * - 1
     - ``analyze``: a file has errors. ``run``: a failed assertion or report at the exit severity,
       or a runtime error.
   * - 2
     - Usage, library or elaboration error.
   * - 3
     - Internal error: please report it.
   * - N
     - ``run``: ``stop(N)`` or ``finish(N)`` with N from 1 to 125.
   * - 130
     - ``run``: stopped by Ctrl-C or SIGTERM.

Scripting
---------

A script or test runner calls ``yavs`` directly with an argument list, without a shell. Reports of
the simulation go to standard output and diagnostics to standard error; never ignore a non-zero
exit status.

.. code-block:: python

    import subprocess

    def yavs(*args):
        return subprocess.run(["yavs", *args], capture_output=True, text=True)

    assert yavs("analyze", "--work=lib:out/lib", "hello_tb.vhd").returncode == 0
    result = yavs("run", "--work=lib:out/lib", "-gEXPECTED=3", "hello_tb")
    print(result.returncode, result.stdout)

An argument ``@FILE`` is replaced by the arguments in FILE, one per line, taken as written (no
quoting). Use it for long file lists and long generic values.

Environment variables
---------------------

None is needed.

* ``NO_COLOR``: no colour in diagnostics.
* ``YAVS_STD_CACHE=DIR|off``: like ``--std-cache``.
* ``YAVS_WAVE_FLUSH=5s``: also write a waveform block every 5 seconds of run time
  (:doc:`waveforms`).

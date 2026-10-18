Command line
============

yavs works like the other command line simulators: analyze the files into libraries, then
elaborate and run a top level. ``yavs --help`` and ``yavs <command> --help`` list every option.

Libraries
---------

A library is a directory, given as ``NAME:PATH``. ``--work`` is the library a command works in,
``--map`` adds another one (repeatable). A missing or empty directory is an empty library;
``analyze`` creates it. ``std`` and ``ieee`` are always available and cannot be mapped.

Analysis is cheap to repeat: a library records which file holds which unit, and ``elab`` and
``run`` analyze the units the top level needs from source, so there is no compiled library to
keep up to date.

Analyze
-------

.. code-block:: bash

    yavs analyze --std=2019 --work=lib:out/lib pkg.vhd tb.vhd

The files are analyzed in order; the first failing file stops the batch. ``--std`` is the
standard of these files (``2008`` by default). ``-D NAME=VALUE`` defines a conditional analysis
identifier. An argument ``@FILE`` is replaced by the arguments in FILE, one per line.

Elaborate and run
-----------------

.. code-block:: bash

    yavs run --work=lib:out/lib -gWIDTH=8 --stop-time=10ms tb

The top level is ``ENTITY``, ``ENTITY.ARCH`` or a configuration. The design is analyzed against
one revision of ``std`` and ``ieee``, the newest standard of its files, unless ``--std`` says
otherwise.

* ``--exit-severity=failure`` (the default): a report of that severity or higher stops the
  simulation with exit status 1. An ``error`` report below it is printed, the run continues,
  and the exit status is 1 at the end, unless ``ClearVhdlAssert`` (VHDL-2019 ``std.env``)
  cleared the assert counts since.
* ``--threads N`` evaluates processes on N threads (default 1). The output is identical for any
  N. ``--shuffle[=SEED]`` randomizes the process order to find testbenches that depend on it.
* ``--wave out.fst`` records the whole design as an FST file that waveform viewers read. It is
  finished on normal end, on ``--stop-time`` and on Ctrl-C. ``--wave-include GLOB`` and
  ``--wave-exclude GLOB`` (repeatable, matching paths such as ``tb.u1.count``) choose what is
  recorded, and arrays larger than ``--wave-max-array`` (default 65536) are left out with a
  warning. Several delta cycles at one time step give one change with the final value.

Diagnostics
-----------

Errors and warnings have a stable code, such as ``E1100``. ``yavs --explain E1100`` prints its
explanation. ``-Werror`` turns warnings into errors, ``-Werror=CODE``, ``-Wno-CODE`` and
``-WCODE`` change one code. ``--message-format=json`` prints one JSON object per diagnostic,
``--color=never`` turns colour off (``NO_COLOR`` is honoured).

Not implemented yet
-------------------

``--load``, ``--vhpi`` and ``--dap`` are accepted but report that they are not
implemented yet, instead of being ignored.

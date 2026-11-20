Debugging
=========

This page says how to debug a simulation from VS Code or another editor. yavs has a debugger built in. It speaks the Debug Adapter Protocol (DAP), the protocol of the
debug views of VS Code and most other editors. :doc:`debugger-tutorial` walks through a failing test.

.. contents:: On this page
   :local:

VS Code with a test runner
--------------------------

With the extension installed (below) and the Python package ``vunit-yavs``, debugging one test needs no configuration:

.. code-block:: bash

    python run.py lib.tb.test_name --yavs-debug

Set breakpoints in your source files and press F5, or run "yavs: Attach" from the command palette. It attaches to port
4711 and asks for a port only if nothing listens there. Breakpoints in your own files bind and the stack shows them, also
when the test runner compiles rewritten copies of them (location and check preprocessing). When you disconnect or the
run ends, the test is reported as usual.

VS Code
-------

The VS Code extension adds a ``yavs`` debugger. Download ``yavs-debug-<version>.vsix`` from the
`releases of yavs <https://github.com/ru551n/yavs/releases>`_ (from 0.12.0 on they carry the extension of the same
version; before that, the release titled "yavs VS Code debugger"; ``SHA256SUMS`` is attached) and install it:

.. code-block:: bash

    code --install-extension yavs-debug-<version>.vsix

To build it from the source tree instead (``editors/vscode``):

.. code-block:: bash

    cd editors/vscode
    npx @vscode/vsce package --out yavs-debug.vsix
    code --install-extension yavs-debug.vsix

What you can do:

* **Breakpoints** on lines, with a hit count (``3``, ``>= 3``) and a VHDL condition such as
  ``count = 3 and state = IDLE``, ``data(7) = '1'``, ``to_integer(unsigned(d)) > 40`` or ``clk'event and clk = '1'``.
* **Signal breakpoints**: "Break on Value Change" on a signal in the Variables view, or
  "yavs: Break when a signal equals..." to stop when it takes a value.
* **Step** over, into and out of subprograms; step to the next delta or the next time step; run
  for or until a time ("yavs: Run for...", "yavs: Run until..."); pause a running simulation.
* **Inspect**: each process is a thread with its call stack, named by its path (``tb.stim``); one without a
  label by what it is and where, such as ``tb.clk <= ... (tb.vhd:17)`` or ``tb.process (tb.vhd:20)``. The Variables
  view shows locals, signals, ports and generics, the shared variables the process can see (a Shared Variables scope, when there are any;
  a protected object shows as ``protected`` and expands to its variables), and a Hierarchy scope lists the design's instances. Hover a signal or a shared variable
  to see its value. An access value shows what it designates and expands as that does; in a method of a protected type,
  Locals lists the object's variables. A vector (``std_logic_vector``, ``unsigned``, ``bit_vector`` and the like) and a
  string show on one line; read one bit with ``dout(3)``. Records and arrays of vectors expand, and a vector longer than
  256 elements shows its start and keeps its elements below it. A Watch takes a name in scope, a dotted path
  (``tb_fifo.dut.count``, a process variable ``tb_fifo.main.i``), a field, an element, a slice (``rec.x``, ``arr(i)``, ``p.all``,
  ``slv(3 downto 0)``) or an expression (``cnt = 3``). ``:tb_fifo:dut:count`` is accepted too. A name that does not exist
  shows an error with the nearest name as its value; VS Code adds the entry to the Watch view before it asks the server, so
  the entry stays listed until you remove it.
* **Change values**: "Set Value" deposits a signal or writes a variable; "Force a signal...",
  "Deposit a signal value..." and "Release a forced signal..." in the context menu. Values are
  VHDL literals: ``'1'``, ``16#FF#``, ``x"A5"``, ``10 ns``, ``(others => '0')``. A bit-like target needs no quotes: ``1`` for
  ``'1'``, ``0101`` for ``"0101"``; an integer's ``1`` stays 1. A force holds what readers of the signal see (the console's
  ``force NAME VALUE -out`` is for the driving value); a release ends any force. A value the target cannot take is an
  error that names its type and what it takes.
* **Debug Console**: the commands of :doc:`interactive` (``help`` lists them, names in bold). Typing shows VS Code's
  list; Tab takes what all entries have in common and never a first guess, and Tab with no list open opens it. With an
  empty word (``step`` and a space) the list has nothing in common to offer, so a second Tab takes the entry it has
  selected. The first word offers commands only; a signal's name there is still evaluated when you press Enter.

Reports go to the Debug Console as they are made, also from a process that does not suspend, and the session ends with a line giving the exit status of the run. A launch that cannot
start (no such top level, an elaboration error, no Surfer) prints its diagnostic there; a command line yavs refuses or a
crash shows a notification with the command to try in a terminal.

Launch configuration (optional)
-------------------------------

A test run needs no launch configuration. Write one in ``.vscode/launch.json`` of your testbench's workspace to start a run
without a test runner: the extension analyzes the files and starts yavs, with the generics and arguments you give it, and
``yavsPath`` names the executable. The next section has the one for a fixed port:

.. code-block:: json

    {
      "version": "0.2.0",
      "configurations": [
        {
          "type": "yavs",
          "request": "launch",
          "name": "Debug tb",
          "top": "tb",
          "work": "work:${workspaceFolder}/.yavs/work",
          "sources": ["${workspaceFolder}/pkg.vhd", "${workspaceFolder}/tb.vhd"],
          "libraries": [],
          "std": "2008",
          "generics": { "WIDTH": "8" },
          "args": ["--stop-time=1ms"]
        }
      ]
    }

``sources`` are analyzed into ``work`` in order before every session (leave it empty to analyze
yourself), ``libraries`` are more ``NAME:PATH`` mappings, and ``args`` go to ``yavs run``. ``std``
is ``2008`` or ``2019``. Set a breakpoint, pick "Debug tb" in the Run and Debug view and press F5.

Attach to a run started elsewhere
---------------------------------

``yavs run --dap=PORT`` waits for a debugger before it starts the simulation; "yavs: Attach" connects to it. For a
fixed port in ``launch.json``:

.. code-block:: json

    { "type": "yavs", "request": "attach", "name": "yavs: Attach", "port": 4711 }

The Restart button of an attached session runs the same design again from time 0 in the same run, keeping the breakpoints,
and stops at the first of them. It compiles nothing: to run changed sources, start the run again. A run with ``--vhpi`` or
``--load`` cannot restart: a notification says so, and the session stays.

A run compiled from rewritten copies of your files can show the originals: ``--source-map=COMPILED=ORIGINAL``
(repeatable; the copy must keep the lines of the original). Paths are matched as files, so a breakpoint or a map entry
spelled through ``..`` or a symlink still finds its file, whether or not it exists on disk. The breakpoints of a file
are one list however the client spells its path: a later ``setBreakpoints`` for the file replaces them.

"Add to Wave" (context menu of the Variables and Watch views and of a selection or word in a VHDL file) adds a signal or a
process variable to the waveforms of a session with ``--surfer``, with its history since time 0; the item appears only in
such a session. A name in the editor is resolved where the selected frame sees it, and a message says when it cannot be
shown, such as a local of a subprogram, when Surfer does not answer because its window is not drawn, or when it is gone
(:doc:`waveforms`).

Add ``--surfer`` to follow the waveforms in Surfer, the cursor at each stop (:doc:`waveforms`). ``--wave=out.fst`` alone
writes the file at every stop, so any viewer that reloads it shows the run up to the stop time, with the values the
Variables view shows. Variables of other processes than the stopped one that changed in this time step reach the file when
it ends.

The Debug Console takes the commands of the :doc:`interactive` too (``run 10 ns``, ``restart``,
``break list``, ``drivers q``, ``trace x q``); the :doc:`console-reference` lists them all.

The context menu of a signal in the Variables view has "Show Drivers". The command
"yavs: Find Signals or Variables..." searches the hierarchy, lists matches in a quick pick, and adds
what you choose to the wave, or else examines it. The Breakpoints view has filters that stop at a report of a
severity with the stack intact: "Assertion: failure", "Assertion: error", "Assertion: warning" and
"Report: note".

Other editors
-------------

``yavs run --dap=PORT`` listens on ``127.0.0.1:PORT`` (``--dap=0`` picks a free port and prints
it); ``--dap=stdio`` talks DAP on standard input and output for clients that start the debugger
themselves. The VS Code commands are custom requests (``yavs/force``, ``yavs/runFor``,
``yavs/hierarchy`` and others) that any client can send; they are specified in the
`command line contract <https://github.com/ru551n/yavs/blob/main/docs/cli-contract.md>`__.

Limits
------

* A debugger session runs the interpreter on one thread, so every session behaves the same;
  ``--threads`` and ``--tier`` have no effect there.
* One driver per run: ``--dap`` cannot be combined with ``--interactive`` or ``--do``, and ``--yavs-debug`` not with
  ``--yavs-interactive``. ``--surfer`` and ``--yavs-surfer`` combine with either.
* ``--cov`` and ``--profile`` cannot be combined with ``--dap``.
* Breakpoint conditions, and expressions in a Watch or ``examine``, call functions of the design and of packages such as
  ``numeric_std`` that only read: pure functions, and impure ones that read signals of their region and ``now``. They run
  apart from the simulation, so a condition cannot change it. A function whose code, or that of what it calls, may write
  (a signal, a shared variable, a protected object, a file, ``std.env.stop``) is refused, naming what and where; so are
  functions with a signal parameter such as ``rising_edge`` (write ``clk'event and clk = '1'``) and functions declared
  in a process. Attributes are those of signals (``'event``, ``'active``,
  ``'last_value``, ``'last_event``, ``'last_active``) and of arrays (``'length``, ``'left``, ``'right``, ``'high``,
  ``'low``, ``'ascending``). A variable of a subprogram has no known type at a stop, so an ``unsigned`` or ``signed`` one
  whose leftmost bit is 1 needs a bit string. A literal that cannot be a value of a signal is refused when the breakpoint
  is set. The full list is in :doc:`console-reference` ("Conditions").
* A value behind an access value and a variable of a protected object are shown, not written, from the Variables view. A
  shared variable of a protected type is seen in the frames of its methods, not from outside.

See also
--------

* :doc:`debugger-tutorial`: a step-by-step walk through a failing test, with runnable files.
* :doc:`interactive`: the same from a terminal prompt.
* :doc:`console-reference`: every console command.
* :doc:`vunit`: debug one test of a run script.
* :doc:`waveforms`: waves while debugging.

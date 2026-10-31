Debugging
=========

This page says how to debug a simulation from VS Code or another editor. yavs has a debugger built in. It speaks the Debug Adapter Protocol (DAP), the protocol of the
debug views of VS Code and most other editors.

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
`releases of yavs <https://github.com/ru551n/yavs/releases>`_ (the releases titled "yavs VS Code
debugger"; ``SHA256SUMS`` is attached) and install it. yavs must be on ``PATH``, or set
``yavsPath``:

.. code-block:: bash

    code --install-extension yavs-debug-<version>.vsix

To build it from the source tree instead (``editors/vscode``):

.. code-block:: bash

    cd editors/vscode
    npx @vscode/vsce package --out yavs-debug.vsix
    code --install-extension yavs-debug.vsix

Add a launch configuration to ``.vscode/launch.json`` of your testbench's workspace:

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

What you can do:

* **Breakpoints** on lines, with a hit count (``3``, ``>= 3``) and a VHDL condition such as
  ``count = 3 and state = IDLE`` or ``data(7) = '1'``.
* **Signal breakpoints**: "Break on Value Change" on a signal in the Variables view, or
  "yavs: Break when a signal equals..." to stop when it takes a value.
* **Step** over, into and out of subprograms; step to the next delta or the next time step; run
  for or until a time ("yavs: Run for...", "yavs: Run until..."); pause a running simulation.
* **Inspect**: each process is a thread with its call stack; the Variables view shows locals,
  signals, ports and generics, and a Hierarchy scope lists the design's instances. Hover a signal
  to see its value.
* **Change values**: "Set Value" deposits a signal or writes a variable; "Force a signal...",
  "Deposit a signal value..." and "Release a forced signal..." in the context menu. Values are
  VHDL literals: ``'1'``, ``16#FF#``, ``x"A5"``, ``10 ns``, ``(others => '0')``.

Reports go to the Debug Console, and the session ends with the exit status of the run.

Attach to a run started elsewhere
---------------------------------

``yavs run --dap=PORT`` waits for a debugger before it starts the simulation; "yavs: Attach" connects to it. For a
fixed port in ``launch.json``:

.. code-block:: json

    { "type": "yavs", "request": "attach", "name": "yavs: Attach", "port": 4711 }

A run compiled from rewritten copies of your files can show the originals: ``--source-map=COMPILED=ORIGINAL``
(repeatable; the copy must keep the lines of the original).

Add ``--surfer`` to follow the waveforms in Surfer, the cursor at each stop (:doc:`waveforms`). ``--wave=out.fst`` alone
flushes the file at every stop, so any viewer that reloads it shows the run up to the breakpoint.

The Debug Console takes the commands of the :doc:`interactive` too (``run 10 ns``, ``restart``,
``break list``).

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
* ``--cov`` and ``--profile`` cannot be combined with ``--dap``.
* Breakpoint conditions cannot call functions or use attributes.

See also
--------

* :doc:`interactive`: the same from a terminal prompt.
* :doc:`vunit`: debug one test of a run script.
* :doc:`waveforms`: waves while debugging.

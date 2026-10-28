Debugging
=========

yavs has a debugger built in. It speaks the Debug Adapter Protocol (DAP), the protocol of the
debug views of VS Code and most other editors.

VS Code
-------

The extension in ``editors/vscode`` of the source tree adds a ``yavs`` debugger. Build and install
it once (yavs must be on ``PATH``, or set ``yavsPath``):

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

Debug one test of a test runner
-------------------------------

``yavs run --dap=PORT`` waits for a debugger before it starts the simulation, so a test runner can
start one test and VS Code can attach to it:

1. Add an attach configuration (the snippet "yavs: Attach to yavs" inserts it):

   .. code-block:: json

       { "type": "yavs", "request": "launch", "name": "Attach to yavs", "port": 4711 }

2. Start exactly one test so that the runner passes ``--dap=4711`` to ``yavs run`` (as an extra
   simulator argument). It prints ``yavs: waiting for a debugger on port 4711``. With the runner
   package ``vunit-yavs`` (``python/vunit-yavs`` in the source tree), ``python run.py
   lib.tb.test_name --yavs-debug`` does this and prints where to attach.
3. Pick "Attach to yavs" and press F5. The run stops at your breakpoints; when you disconnect or
   the run ends, the runner reports the test as usual.

Add ``--wave=out.fst`` to follow the waveforms next to the session: the file is flushed at every
stop, so a viewer that reloads it shows the run up to the breakpoint.

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

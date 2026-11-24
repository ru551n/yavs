Waveforms
=========

This page says how to record a run in a waveform file, how to choose what is recorded, and how to watch waves
while the simulation runs.

.. code-block:: bash

    yavs run --work=work:build/work --wave=tb.fst tb

``--wave=FILE.fst`` records every signal and port of the design in an FST file, which common
waveform viewers open. Two runs of the same design write identical files, at any ``--threads``.

.. contents:: On this page
   :local:

Choosing what is recorded
-------------------------

.. code-block:: bash

    yavs run --work=work:build/work --wave=tb.fst --wave-exclude='tb.u_ddr.**' tb

* ``--wave-include=GLOB`` and ``--wave-exclude=GLOB`` (repeatable) select by path, such as
  ``tb.u1.count``. Matching ignores case; ``*`` matches within one level, ``**`` across levels and
  ``?`` one character; a pattern that matches a scope covers everything below it. Signals left
  out cost nothing while simulating.
* ``--wave-variables`` also records process variables (in a scope named after the process, such as
  ``tb.main.count``) and shared variables. A variable shows its value at the end of each time step. A debug session
  (``--dap``, ``--interactive``, ``--do``) that records a wave does this without the flag. A local of a subprogram is never
  recorded: it exists only during a call. A variable that holds an access value or a protected object is not either; at a stop, ``examine`` shows it.
* ``--wave-max-array=N`` (default 65536) leaves out arrays with more elements, such as large
  memories, with warning ``W0802``.

What the file shows
-------------------

* The hierarchy of the elaborated design: instances (with their ``entity(arch)``), blocks,
  generate scopes (``g(3)`` for a for-generate index) and package signals (``work.pkg.sig``).
* ``std_logic`` and ``bit`` vectors as vectors of characters (``U``, ``X``, ``Z``...), other
  enumerations by their literal names, integers, ``time`` and ``real`` as numbers. Records and
  other arrays are scopes with one entry per field or element.
* FST has no delta cycles: several deltas at one time give one change with the final value.
* Under ``--cycle`` the signals inside the cycle region are recorded from its own values at the end of each
  time step, two-state with ``--two-state`` (:doc:`two-state`); a memory of integers it writes at run-time indices is left out (W0802), and
  ``--wave-variables`` is refused. :ref:`cycle-waves` has the details.

The file is finished at the end of the run, at ``--stop-time`` and on Ctrl-C. A run killed hard
(a crash, SIGKILL) leaves the file readable up to its last written block; setting the environment
variable ``YAVS_WAVE_FLUSH=5s`` also writes a block every 5 seconds of run time.

Watching waves while debugging
------------------------------

``--wave`` works in a debugger session too: at every stop the file holds all completed time
steps, so a viewer that reloads the file shows the run up to the breakpoint (:doc:`debugging`).

Watching in Surfer
------------------

``--surfer`` needs 0.12.0 or later. It shows the waveform in the `Surfer <https://surfer-project.org>`__ viewer (0.6.0 or later) while the run writes it:

.. code-block:: sh

    yavs run --work=work:build/work tb --surfer                 # starts surfer from PATH
    yavs run --work=work:build/work tb --surfer=127.0.0.1:54321 # connects to a running Surfer

To connect, start Surfer's WCP server first (its command ``wcp_server_start``, or ``[wcp] autostart = true`` in its
configuration). Nothing starts or connects to Surfer without this flag.

Without ``--wave`` the file is ``<top>.fst`` next to the work library directory (``--work=work:build/work`` gives
``build/tb.fst``), and its path is printed. Surfer shows the signals of the top level, with records and arrays one level
down, or everything ``--wave-include`` chose, at most 200 of them on their own. It reloads the file every second:
``--surfer-refresh=500ms`` changes that, and wins over ``YAVS_WAVE_FLUSH``. At the end it shows the whole run and stays open.

In a debug session (``--dap``, ``--interactive``, ``--do``) each stop syncs the file at once: the cursor moves to the stop, a
breakpoint leaves a marker, and the signals of the stopped process are coloured. Surfer's "go to declaration" prints the
file and line, and opens it in VS Code. At the console, ``wave add NAME...`` adds signals, variables of the stopped process
or scopes, with their history since time 0, and ``wave cursor [TIME]`` moves the cursor. In VS Code, "Add to Wave" in the
context menu of the Variables and Watch views, and of a selection or a word in a VHDL file, does the same.

The simulation never waits for the viewer, and its output and exit status do not depend on it: losing Surfer during a run is
warning ``W0131``, also under ``-W error``. Starting it or connecting to it is checked first (``E0130``).

Keep Surfer's window in view. Surfer handles commands only while it draws its window, has no setting to do so otherwise, and
a covered or minimized window may not be drawn. After 5 s without an answer the Debug Console or the terminal prompt says so,
and ``wave add`` answers at once with the same reason; what it names is added when Surfer answers. Bring the window to the
front, or keep it beside the editor, and the view catches up by itself. Commands Surfer refuses are told one by one and the
view goes on following. When the connection is gone, because Surfer was closed or crashed, that is told when it happens, and
the view stays as it was: run again with ``--surfer`` to watch. The waveform file is written either way.

A signal shows in Surfer under its last name: ``x`` for ``tb.r.x``, ``(0)`` for ``tb.m(0)``, and its times in femtoseconds, since
the file has that resolution.

Keep the signal list
--------------------

The list a session shows in the viewer survives a restart and a new run: when a session ends, yavs
saves it, and the next ``--surfer`` run of the same testbench shows it instead of the default.
Names that no longer exist are left out. ``wave save [FILE]`` and ``wave load [FILE]`` do the same
at the console, for this testbench's list or a file of your own (:doc:`on-disk`). The list holds
names only, not colours or the order of groups, and an entry added in the viewer whose short name
fits several signals is left out.

See also
--------

* :doc:`debugging`: stop at a breakpoint and look at the waves up to it.
* :doc:`cli-reference`: every ``--wave`` option.
* :doc:`environment`: ``YAVS_WAVE_FLUSH``.

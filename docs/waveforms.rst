Waveforms
=========

.. code-block:: bash

    yavs run --work=work:build/work --wave=tb.fst tb

``--wave=FILE.fst`` records every signal and port of the design in an FST file, which common
waveform viewers open. Two runs of the same design write identical files, at any ``--threads``.

Choosing what is recorded
-------------------------

.. code-block:: bash

    yavs run --work=work:build/work --wave=tb.fst --wave-exclude='tb.u_ddr.**' tb

* ``--wave-include=GLOB`` and ``--wave-exclude=GLOB`` (repeatable) select by path, such as
  ``tb.u1.count``. Matching ignores case; ``*`` matches within one level, ``**`` across levels and
  ``?`` one character; a pattern that matches a scope covers everything below it. Signals left
  out cost nothing while simulating.
* ``--wave-variables`` also records process variables (in a scope named after the process, such as
  ``tb.main.count``) and shared variables. A variable shows its value at the end of each time step.
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

The file is finished at the end of the run, at ``--stop-time`` and on Ctrl-C. A run killed hard
(a crash, SIGKILL) leaves the file readable up to its last written block; setting the environment
variable ``YAVS_WAVE_FLUSH=5s`` also writes a block every 5 seconds of run time.

Watching waves while debugging
------------------------------

``--wave`` works in a debugger session too: at every stop the file holds all completed time
steps, so a viewer that reloads the file shows the run up to the breakpoint (:doc:`debugging`).

Watching in Surfer
------------------

``--surfer`` shows the waveform in the `Surfer <https://surfer-project.org>`__ viewer (0.6.0 or later) while the run writes it:

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
file and line, and opens it in VS Code. At the console, ``wave add NAME...`` adds signals or scopes and ``wave cursor [TIME]``
moves the cursor.

The simulation never waits for the viewer, and its output and exit status do not depend on it: losing Surfer during a run is
warning ``W0131``, also under ``-W error``. Starting it or connecting to it is checked first (``E0130``).

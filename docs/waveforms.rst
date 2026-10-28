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

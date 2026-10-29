Running faster
==============

The defaults are the fast settings; this page says what they do and what you can change.

Tiers
-----

* ``--tier=auto`` (the default in the release archives) starts the run in the interpreter and
  compiles the functions that become hot on a background thread while the simulation goes on.
* ``--tier=clift`` compiles everything before time 0. It can pay off for a long run of a
  compute-heavy design.
* ``--tier=interp`` runs the interpreter alone; it is the default of a source build without a
  compiled tier (:doc:`install`).

The output is identical on every tier.

Warm runs
---------

yavs keeps what a run analyzed and compiled in ``yavs-cache/`` next to the work library. The first
run of a design notes it, the second stores it, and later runs of the same design, and the other
test cases of the same testbench, reuse it. Short tests then mostly skip analysis and compilation.

* Keep ``yavs-cache/`` between runs (in CI, cache the library directories).
* An edited file, a different yavs or different options make a new entry; nothing has to be
  cleaned by hand. ``--no-cache`` turns the cache off for one run, and
  ``yavs cache clean --work=NAME:PATH`` empties it.
* The analyzed ``std`` and ``ieee`` packages are kept once per user (in ``yavs/std`` of your cache
  directory; ``--std-cache=DIR`` or ``off`` changes that).
* Analyze the files of one library and standard in one ``analyze`` call rather than one call per
  file: the call shares its work.

Threads
-------

``--threads=N`` evaluates processes on N threads with output identical to one thread. The
processes of a delta cycle go to the threads only when their estimated work is worth the
hand-over; smaller groups run on one thread. It helps only large, flat designs where many
processes wake in the same delta cycle; leave it at 1 (the default) unless you measured a gain. A test runner that already
runs tests in parallel should leave it at 1, unless it runs a token server and names it in ``YAVS_TOKENS``:
the extra threads then run only on tokens it lends, so the simulations share one CPU budget, on
Linux and Windows alike (for VUnit, see :ref:`vunit-token-server`). Measurements and the token server are in
`parallel.md <https://github.com/ru551n/yavs/blob/main/docs/parallel.md>`__.

Finding where the time goes
---------------------------

* ``--time-report`` prints the wall and CPU time of analysis, elaboration, initialization and
  simulation when the run ends.
* ``--profile[=FILE]`` samples the run and prints the busiest processes, instances, subprograms
  and source lines (with ``FILE``, also as JSON). Ctrl-C prints the profile of the time so far.

How yavs compares with other simulators is on the :doc:`performance` page.

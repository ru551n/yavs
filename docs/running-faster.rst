Running faster
==============

This page is for making a simulation faster. The defaults are the fast settings; it says what they do and what
you can change.

.. contents:: On this page
   :local:

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

* Keep ``yavs-cache/`` between runs (in CI, cache the library directories: :doc:`ci`).
* An edited file, a different yavs or different options make a new entry; nothing has to be
  cleaned by hand. ``--no-cache`` turns the cache off for one run, and
  ``yavs cache clean --work=NAME:PATH`` empties it.
* The analyzed ``std`` and ``ieee`` packages are kept once per user (:doc:`on-disk`).
* Analyze the files of one library and standard in one ``analyze`` call rather than one call per
  file: the call shares its work.

Threads
-------

``--threads=N`` helps only large, flat designs where many processes wake in the same delta cycle. Leave it at 1
(the default) unless you measured a gain: :doc:`threads`.

Finding where the time goes
---------------------------

* ``--time-report`` prints the wall and CPU time of analysis, elaboration, initialization and
  simulation when the run ends.
* ``--profile[=FILE]`` samples the run and prints the busiest processes, instances, subprograms
  and source lines (with ``FILE``, also as JSON). Ctrl-C prints the profile of the time so far.

See also
--------

* :doc:`threads`: more threads, and when they help.
* :doc:`on-disk`: where the cache lives.
* :doc:`performance`: how yavs compares with other simulators.

Threads and the token server
============================

This page says when more threads make a simulation faster, how to ask for them, and how parallel tests
share CPU cores through the token server. Most designs do not gain: measure first, or let
``--threads=auto`` find out.

.. contents:: On this page
   :local:

What threads do
---------------

``yavs run --threads=N`` lets one simulation use N threads, a whole number of at least 1. The default
is 1. ``--threads=auto`` decides while the run goes (`Letting yavs decide`_).

* Only the evaluation of VHDL processes is shared, and under ``--cycle`` the compilation of the region
  before time 0 (:doc:`cycle-mode`). Analysis, elaboration and most of the rest of a run stay on one thread.
* The output, reports, waves, coverage and exit status are identical at any thread count.
* The processes that wake in one delta cycle go to the threads only when yavs estimates that the work
  is worth the hand-over. A smaller group stays on the main thread, as with ``--threads=1``. yavs measures
  while it runs, and keeps the work on one thread for a design whose groups turn out no faster.
* A process that touches files, shared variables or protected objects runs on the main thread, in the
  same order as on one thread.
* ``--tier=auto`` compiles while the simulation waits (``--tier-compile=wait``, the default). A tier-up thread
  still prepares functions alongside the run, so a run can use somewhat more than ``--threads`` cores for short
  periods, and ``--tier=interp`` starts no such thread. With ``--tier-compile=background`` it compiles on that
  thread alongside the run, at the lowest priority of the system, not counted in ``--threads``.
* A debugger or console session runs on one thread, and ``--threads`` is then ignored with warning
  ``W0109``.

When threads help
-----------------

They help when many instances do real work on the same clock edge:

* generate loops of processing elements, multi-lane or multi-channel datapaths, MAC or systolic arrays;
* processes with heavy work per clock edge, such as wide arithmetic, vector loops or records;
* large flat RTL with many independent clocked processes.

They do not help, and cost a few percent, when:

* one or a few large processes do most of the work, as in a processor model;
* the test is testbench-style, with a stimulus process, a checker and a small design;
* long chains of delta cycles wake only a few processes each;
* the processes must run in order: file or text I/O, shared variables, protected objects.

Letting yavs decide
-------------------

.. code-block:: bash

    yavs run --work=work:build/work tb --threads=auto

With ``--threads=auto`` yavs uses more threads only while they make the run faster:

* It may use as many threads as the CPUs the run is allowed on (a ``taskset`` or container limit counts),
  at most 8. The threads start only when a group of processes is first worth sharing.
* A design with fewer than 8 processes that can run in parallel stays on one thread for the whole run.
* Otherwise yavs tries the threads from 10 ms into the run and measures in short windows of about 25 ms what
  they save. When they save too little,
  it goes back to one thread, exactly as with ``--threads=1``, and tries again later: after 1 second, then after
  2, 4, up to 16 seconds. A design whose threads gained tries again after 0.1 s, so a busy part later in the
  test is found again.
* It tries half as many threads now and then and keeps the faster count, half unless all are clearly faster:
  two hardware threads of one core can be slower than one.
* It remembers what the last run of the same command line found, in the work library's cache (not with
  ``--no-cache``): a test that did not gain starts on one thread, one that gained starts with the threads on.
  The arguments in a response file count as given on the command line. In a ``runner_cfg`` generic the seed and
  output path a test runner gives each run do not count, so a VUnit test is the same test in every run, whatever
  its seed. The compiled tier's profile of the last run is kept the same way.
* ``--verbose`` says what it decided at the end of the run.
* The output is the same whatever it decides.

Measured on small benchmark designs (2026-11-07, 4 cores and their second hardware threads, a busy host):
designs of thousands of small processes on one clock got 93 to 102 percent of the speedup of the best fixed
``--threads`` once the last run was remembered, and designs that do not gain ran within 1 percent of
``--threads=1``. The first run of a design, with nothing remembered, does worse on some: 89 and 80 percent on two
designs with compiled processes, or 94 and 85 percent against fixed runs that were first runs too (the compiled tier
keeps its own profile per command line). On 8 cores and their second hardware threads the first runs of three
such designs reached 89 to 96 percent of the best fixed ``--threads``, measured against first runs too. A test of a few
milliseconds runs on one thread. The numbers are in
`parallel.md <https://github.com/ru551n/yavs/blob/main/docs/parallel.md>`__. Measure your own longest test if
you depend on it (`How to decide`_).

How to decide
-------------

Run the longest test alone and compare, on an otherwise idle machine:

.. code-block:: bash

    yavs run --work=work:build/work tb --threads=1 --time-report
    yavs run --work=work:build/work tb --threads=4 --time-report

``--time-report`` prints the wall and CPU time of each phase. If it is at least about 1.3 times faster with 4
threads, they are worth using. Most designs gain little beyond 4: the flat RTL design that gained most ran 2.39 times
faster at 4 threads and 3.25 at 8, whole run, measured 2026-10-28. The measured gains per kind of design are in
`parallel.md <https://github.com/ru551n/yavs/blob/main/docs/parallel.md>`__ in the repository.

Regressions: threads or parallel tests
--------------------------------------

Running tests side by side uses cores better than threads inside one test. yavs alone does not share the CPU
between simulations: with ``-p 8`` and ``--threads=8`` the threads of all runs add up. A test runner's token
server shares it, and the ``vunit-yavs`` package starts one by default (`With VUnit`_).

The token server
----------------

With ``-p N`` a test runner runs N tests side by side. Near the end of a regression most of the CPUs
are idle while a few long simulations finish alone, and with ``-p 1`` all but one are. The token server
lends the idle CPUs to those simulations as extra threads, so they end sooner, without more threads in
all than the machine has.

* It is a small server of the test runner on ``127.0.0.1``. The ``vunit-yavs`` package has one
  (next section). It works on Linux and Windows alike.
* yavs finds it through the environment variable ``YAVS_TOKENS``, which the runner sets. You do not set
  it by hand. A value yavs cannot read is error ``E0128``: unset it. A server it cannot reach, often a value
  left over from a runner that ended, is warning ``W0118``, and the run has one thread.
* ``--threads=N`` is the cap. The simulation owns its main thread and can use up to N - 1 more, as tokens
  are lent. It starts its extra threads with the first token. With no token lent, it runs as with
  ``--threads=1``.
* yavs asks for a token only when a group of processes is worth sharing, or, under ``--cycle``, while it
  compiles the region (:doc:`cycle-mode`), and never waits for one. It gives tokens back when the groups
  stay small, and all of them when the simulation ends, however it ends.
* The output does not depend on the tokens.

With VUnit
----------

The ``vunit-yavs`` package starts the token server for every run, at any ``-p``, with one slot for each CPU the
run may use (a ``taskset`` or CPU affinity counts): one slot per running test, the rest lent. A run of ``-p N``
with N at or above the CPU count lends nothing.

.. code-block:: bash

    python run.py -p 8
    python run.py -p 8 --yavs-max-threads 4
    python run.py -p 8 --yavs-no-jobserver

Under the server every simulation runs with ``--threads=auto``: it asks for tokens only while its threads
gain, and gives them back when they do not. ``--yavs-max-threads N`` gives every simulation ``--threads=N``
instead (``--yavs-max-threads auto`` is the default); those threads past the first also run only on lent tokens.
A test can have its own value with ``vu.set_sim_option("yavs.max_threads", 2)`` or ``"auto"``; with ``1`` it
takes no tokens and still counts as a running test. If the server cannot start (a blocked local socket), the run
prints a warning and every simulation runs on one thread.

``--yavs-no-jobserver`` runs without the server: then nothing is passed unless ``--yavs-max-threads`` or
``yavs.max_threads`` is set, and ``--yavs-max-threads N`` gives each simulation N threads of its own, whatever
the others use. A run script with a command line of its own sets ``VUNIT_YAVS_JOBSERVER=off`` for the same, and
``VUNIT_YAVS_MAX_THREADS`` for ``--yavs-max-threads``. ``--yavs-jobserver``, from the time the server was not the
default, is still accepted and changes nothing.

A run never keeps more threads busy than it has slots once it has settled. A test that starts while every slot
is lent starts at once, so the total can briefly exceed the CPU count.

The gain is on designs whose threads gain when run alone (`How to decide`_): in the tail of a regression and
in a run of ``-p 1``. Processor models and testbench-style tests ask for no tokens after a short trial. The
measurements are in `parallel.md <https://github.com/ru551n/yavs/blob/main/docs/parallel.md>`__.

See also
--------

* :doc:`running-faster`: tiers, the cache and profiling, which usually matter more.
* :doc:`vunit`: running tests with VUnit.
* :doc:`performance`: how yavs compares with other simulators.
* :doc:`environment`: ``YAVS_TOKENS``.

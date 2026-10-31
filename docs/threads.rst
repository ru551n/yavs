Threads and the token server
============================

This page says when more threads make a simulation faster, how to ask for them, and how parallel tests
share CPU cores through the token server. Most designs do not gain: measure first.

.. contents:: On this page
   :local:

What threads do
---------------

``yavs run --threads=N`` lets one simulation use N threads. The default is 1.

* Only the evaluation of VHDL processes is shared. Analysis, elaboration and most of the rest of a run
  stay on one thread.
* The output, reports, waves, coverage and exit status are identical at any thread count.
* The processes that wake in one delta cycle go to the threads only when yavs estimates that the work
  is worth the hand-over. A smaller group stays on the main thread, as with ``--threads=1``. yavs measures
  while it runs, and keeps the work on one thread for a design whose groups turn out no faster.
* A process that touches files, shared variables or protected objects runs on the main thread, in the
  same order as on one thread.
* The compile thread of ``--tier=auto`` is one more background thread, not counted in ``--threads``.
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

How to decide
-------------

Run the longest test alone and compare, on an otherwise idle machine:

.. code-block:: bash

    yavs run --work=work:build/work tb --threads=1 --time-report
    yavs run --work=work:build/work tb --threads=4 --time-report

``--time-report`` prints the wall and CPU time of each phase. If it is at least about 1.3 times faster with 4
threads, they are worth using. Most designs gain little beyond 4. The measured gains per kind of design are in
`parallel.md <https://github.com/ru551n/yavs/blob/main/docs/parallel.md>`__ in the repository.

Regressions: threads or parallel tests
--------------------------------------

Running tests side by side uses cores better than threads inside one test. Nothing shares the CPU
between simulations by itself: with ``-p 8`` and ``--threads=8`` the threads of all runs add up. So leave
``--threads`` at 1 when a runner already runs tests in parallel, or use the token server.

The token server
----------------

With ``-p N`` a test runner runs N tests side by side. Near the end of a regression most of those N
slots are idle while a few long simulations finish alone. The token server lends the idle slots to
those simulations as extra threads, so the tail ends sooner.

* It is a small server of the test runner on ``127.0.0.1``. The ``vunit-yavs`` package has one
  (next section). It works on Linux and Windows alike.
* yavs finds it through the environment variable ``YAVS_TOKENS``, which the runner sets. You do not set
  it by hand. A value yavs cannot read is error ``E0128``: unset it. A server it cannot reach, often a value
  left over from a runner that ended, is warning ``W0118``, and the run has one thread.
* ``--threads=N`` is the cap. The simulation owns its main thread and can use up to N - 1 more, as tokens
  are lent. It starts its extra threads with the first token. With no token lent, it runs as with
  ``--threads=1``.
* yavs asks for a token only when a group of processes is worth sharing, and never waits for one. It gives
  tokens back when the groups stay small, and all of them when the simulation ends, however it ends.
* The output does not depend on the tokens.

With VUnit
----------

Turn it on with ``--yavs-jobserver`` together with ``-p`` above 1. With ``-p 1`` there is nothing to lend
and the flag does nothing:

.. code-block:: bash

    python run.py -p 8 --yavs-jobserver
    python run.py -p 8 --yavs-jobserver --yavs-max-threads 4

``--yavs-max-threads N`` caps the threads of one simulation; the default with the server is the ``-p``
value, at most 8, since more threads buy little and each costs memory in every simulation. A test can have its
own cap with ``vu.set_sim_option("yavs.max_threads", 2)``. Without the server ``--yavs-max-threads N`` simply runs
each simulation with ``--threads=N``. If the server cannot start (a blocked local socket), the run prints a
warning and every simulation uses one thread. A run script with a command line of its own can set
``VUNIT_YAVS_MAX_THREADS`` instead of the flag.

A run never keeps more than N threads busy once it has settled. A test that starts while every slot is
lent starts at once, so the total can briefly exceed N.

Turn the server on only for a regression whose longest tests gain from threads when run alone (`How to
decide`_): the gain is in the tail, and the server gives nothing to a tail of processor models or
testbench-style tests. The measurements are in
`parallel.md <https://github.com/ru551n/yavs/blob/main/docs/parallel.md>`__.

See also
--------

* :doc:`running-faster`: tiers, the cache and profiling, which usually matter more.
* :doc:`vunit`: running tests with VUnit.
* :doc:`performance`: how yavs compares with other simulators.
* :doc:`environment`: ``YAVS_TOKENS``.

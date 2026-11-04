Running faster
==============

This page is for making a simulation faster. The defaults are the fast settings; it says what they do and what
you can change.

.. contents:: On this page
   :local:

Tiers
-----

* ``--tier=auto`` (the default in the release archives) starts the run in the interpreter and
  compiles the functions that become hot on a background thread, with the bundled Cranelift code generator, while the
  simulation goes on.
* ``--tier=clift`` compiles everything before time 0. It can pay off for a long run of a
  compute-heavy design.
* ``--tier=interp`` runs the interpreter alone; it is the default of a source build without a
  compiled tier (:ref:`installation`).

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
* The cache has a size limit per library directory, and a large suite may use more than a GB of disk
  (:doc:`on-disk`).
* The analyzed ``std`` and ``ieee`` packages are kept once per user (:doc:`on-disk`).
* Analyze the files of one library and standard in one ``analyze`` call rather than one call per
  file: the call shares its work.
* Relative and absolute names of a file share what ``analyze`` kept: ``elab`` and ``run`` reuse it either
  way. A name that goes through ``..`` or a symlink counts as another name until it is analyzed: the units
  of that file, and those that depend on them, are analyzed once more under it. Only the time differs.

.. _huge-pages:

Huge pages
----------

A long run of a large design spends part of its time on address translation. On Linux with glibc, yavs asks the
kernel for transparent huge pages (2 MiB) for its heap and for compiled code, and keeps asking as the heap grows. On
a design with a heap of a few hundred MiB this roughly halves the data-TLB misses; the effect on run time is a few percent
at most, and on a short run there is none. The memory in use differs by a few MiB. Nothing is asked for when the
system does not allow it, and nothing has to be set.

* The kernel must have transparent huge pages in ``madvise`` mode, which most distributions use by default. Check
  with ``cat /sys/kernel/mm/transparent_hugepage/enabled``: ``always`` and ``madvise`` give the pages (``always`` without
  yavs asking), ``never`` gives none and yavs leaves the heap as it is. Changing that setting is the
  administrator's choice; yavs does not work around it.
* ``YAVS_HUGE_HEAP=0`` stops yavs from asking for them on the heap (:doc:`environment`); ``YAVS_CLIFT_HUGE=0`` and
  ``YAVS_LLVM_HUGE=0`` do the same for compiled code.
* ``GLIBC_TUNABLES`` is never written by yavs. If you set ``glibc.malloc.top_pad``, ``trim_threshold``, ``mmap_threshold`` or ``hugetlb``
  (or the older ``MALLOC_TOP_PAD_``, ``MALLOC_TRIM_THRESHOLD_``, ``MALLOC_MMAP_THRESHOLD_``), yavs leaves the allocator to you and asks for nothing on the heap;
  ``glibc.malloc.hugetlb=1`` (glibc 2.35 and later) gives the effect for the whole heap, and is not needed otherwise. Other tunables change nothing here.
* Other systems (musl, macOS, Windows) run as before, on ordinary pages.

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
* ``--activity[=FILE]`` counts instead of measuring: the processes that ran most often and the signals that changed most
  often (:doc:`cli`).

See also
--------

* :doc:`threads`: more threads, and when they help.
* :doc:`on-disk`: where the cache lives.
* :doc:`performance`: how yavs compares with other simulators.

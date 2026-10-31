.. _environment:

Environment variables
=====================

Every environment variable that ``yavs`` and ``vunit-yavs`` read or set. You do not need any of
them for normal use, and options on the command line win over them. A test checks this page against
the source, so a variable that is added or removed shows up here.

.. contents:: On this page
   :local:
   :depth: 2


For users
---------

Read by yavs
~~~~~~~~~~~~

.. list-table::
   :header-rows: 1
   :widths: 20 36 14 10 12 8

   * - Variable
     - Purpose
     - Values
     - Default
     - Read by
     - Since
   * - ``YAVS_STD_CACHE``
     - Where the analyzed ``std`` and ``ieee`` packages are kept, as ``--std-cache`` (:doc:`on-disk`). A store that cannot be used is error ``E0126``.
     - a directory, or ``off`` to keep them in each library
     - ``yavs/std`` in your cache directory
     - ``analyze``, ``elab``, ``run``, ``cache clean``
     - 0.9.0
   * - ``YAVS_WAVE_FLUSH``
     - Also end a waveform block after this much run time, so that a killed run leaves more of the file (:doc:`waveforms`). Any other value is error ``E0801``.
     - ``5s``, ``500ms``, ``never``
     - unset: no block by the clock
     - ``run``
     - 0.9.0
   * - ``YAVS_HUGE_HEAP``
     - Set to ``0`` to not ask for transparent huge pages on the heap at start. Linux only.
     - ``0``
     - unset: asks for them
     - every command
     - 0.9.0
   * - ``NO_COLOR``
     - When set and not empty, diagnostics have no colour. ``--color=always`` still forces it.
     - any non-empty value
     - unset
     - every command
     - 0.1.0
   * - ``XDG_CACHE_HOME``
     - Linux: the base of the default ``YAVS_STD_CACHE``. Used only when it is an absolute path.
     - a directory
     - ``$HOME/.cache``
     - ``analyze``, ``elab``, ``run``
     - 
   * - ``HOME``
     - Linux: gives the cache directory when ``XDG_CACHE_HOME`` is not usable. Neither one known is error ``E0126``.
     - a directory
     - 
     - ``analyze``, ``elab``, ``run``
     - 
   * - ``LOCALAPPDATA``
     - Windows: the base of the default ``YAVS_STD_CACHE``. Not usable is error ``E0126``.
     - a directory
     - 
     - ``analyze``, ``elab``, ``run``
     - 

VHDL code can read any variable with ``std.env.getenv`` (VHDL-2019).

``PATH`` finds ``surfer`` for ``yavs run --surfer`` and a plain library name in ``--load`` or ``--vhpi``
goes through the library search path of the operating system (``LD_LIBRARY_PATH`` on Linux).

Set by a test runner, read by yavs
~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~

``vunit-yavs`` sets these for each simulation it starts. Do not set them by hand. A value left over
from a test runner that ended is an error: unset the variable to run without it.

.. list-table::
   :header-rows: 1
   :widths: 20 36 14 10 12 8

   * - Variable
     - Purpose
     - Values
     - Default
     - Read by
     - Since
   * - ``YAVS_TOKENS``
     - The token server that lends CPU slots: with ``--threads`` above 1 the extra threads run only on tokens it lends (:doc:`threads`). A value yavs cannot read is error ``E0128`` and exit status 2. A server it cannot reach is warning ``W0118``, and the run has one thread.
     - ``PORT:SECRET``
     - unset: no server
     - ``run``
     - 0.11.0
   * - ``YAVS_CONSOLE_HOOK``
     - The console hook of a test runner, which adds commands to the console (:doc:`interactive`). A value yavs cannot use is error ``E0703``.
     - ``PORT:SECRET``
     - unset: no hook
     - ``run`` with ``--interactive``, ``--do`` or ``--dap``
     - unreleased

Read by vunit-yavs and VUnit
~~~~~~~~~~~~~~~~~~~~~~~~~~~~

.. list-table::
   :header-rows: 1
   :widths: 20 36 14 10 12 8

   * - Variable
     - Purpose
     - Values
     - Default
     - Read by
     - Since
   * - ``VUNIT_SIMULATOR``
     - Selects the simulator. Set it to ``yavs``; without it VUnit picks the first simulator it finds on ``PATH`` (:doc:`vunit`).
     - ``yavs``
     - first found
     - VUnit
     - 
   * - ``VUNIT_YAVS_PATH``
     - The directory of the ``yavs`` executable.
     - a directory
     - found on ``PATH``
     - VUnit, for ``vunit-yavs``
     - ``vunit-yavs`` 0.10.0
   * - ``VUNIT_YAVS_MAX_THREADS``
     - ``--yavs-max-threads``, for a run script with a command line of its own.
     - a number, at least 1
     - unset
     - ``vunit-yavs``
     - newer than ``vunit-yavs`` 0.11.0
   * - ``VUNIT_VHDL_STANDARD``
     - The default standard of the files of a run script (:doc:`mixing-standards`).
     - ``1993``, ``2008``, ``2019``
     - ``2008``
     - VUnit
     -
   * - ``YAVS``
     - The file name of the executable to look for, read once when the package is imported.
     - a file name
     - ``yavs``
     - ``vunit-yavs``
     - ``vunit-yavs`` 0.10.0

For development and diagnostics
-------------------------------

These are for working on yavs itself and for bug reports. They may change or go away in any release
without a note. "Any value" means the variable only has to be set. "Not ``0``" means set to anything but ``0`` or the empty
string.

Diagnostics of a run
~~~~~~~~~~~~~~~~~~~~

.. list-table::
   :header-rows: 1
   :widths: 22 38 16 10 14

   * - Variable
     - Purpose
     - Values
     - Default
     - Read by
   * - ``YAVS_LOG``
     - Notes on the cache and on library locks, on standard error.
     - ``debug``
     - unset
     - every command
   * - ``YAVS_COUNTERS``
     - Prints the work counters of a run as ``counters: NAME=VALUE`` lines on standard error. ``kinds`` and ``pc`` add the calls compiled code makes into the host.
     - ``1``, ``kinds``, ``pc``
     - unset
     - ``run``
   * - ``YAVS_DUMPIR``
     - Prints the finished IR of each function when it first runs, and why a loop has no fast form.
     - any value
     - unset
     - ``run``
   * - ``YAVS_CENSUS_TOP``
     - Rows per table of the op census (a build with the ``op-census`` feature).
     - a number
     - the table's own
     - ``run``
   * - ``YAVS_VERIFY_IR``
     - Verifies every IR function at once rather than when it first runs. Always on in a debug build.
     - any value
     - unset
     - ``run``
   * - ``YAVS_UNIT_CHECK``
     - ``analyze`` analyzes every file a second time without the unit cache and compares the two.
     - any value
     - unset
     - ``analyze``
   * - ``YAVS_UNIT_DEBUG``
     - Prints the query runs of a failing ``YAVS_UNIT_CHECK`` comparison.
     - any value
     - unset
     - ``analyze``
   * - ``YAVS_CACHE_SALT``
     - Mixed into every cache key, so that a run does not share cache entries with other runs.
     - any text
     - empty
     - every command
   * - ``YAVS_STDLIB``
     - A directory of ``std`` and ``ieee`` sources to use instead of the built-in copy.
     - a directory
     - the built-in copy
     - every command
   * - ``YAVS_SPIN_LIMIT``
     - Jumps a process activation may take before it fails with ``E3009``. Not a positive integer is an internal error.
     - a positive integer
     - ``1000000000``
     - ``run``

Optimiser and kernel switches
~~~~~~~~~~~~~~~~~~~~~~~~~~~~~

Each one turns a fast path off or changes its threshold, to compare a run with and without it.

.. list-table::
   :header-rows: 1
   :widths: 22 38 16 10 14

   * - Variable
     - Purpose
     - Values
     - Default
     - Read by
   * - ``YAVS_NO_IROPT``
     - Turns the IR optimiser off.
     - not ``0``
     - unset
     - ``run``
   * - ``YAVS_NO_CFG``
     - Turns the control flow pass off.
     - not ``0``
     - unset
     - ``run``
   * - ``YAVS_NO_CASETAB``
     - Turns the case table pass off.
     - not ``0``
     - unset
     - ``run``
   * - ``YAVS_NO_DRIVES``
     - Turns the drive pass off.
     - not ``0``
     - unset
     - ``run``
   * - ``YAVS_NO_INLINE``
     - Turns inlining off.
     - not ``0``
     - unset
     - ``run``
   * - ``YAVS_NO_MEMO``
     - Turns the optimiser's memoisation pass off.
     - not ``0``
     - unset
     - ``run``
   * - ``YAVS_NO_SLOTS``
     - Turns the slot pass off.
     - not ``0``
     - unset
     - ``run``
   * - ``YAVS_NO_SPEC``
     - Turns per-instance copies of process code off.
     - not ``0``
     - unset
     - ``run``
   * - ``YAVS_SPEC_AFTER``
     - Activations before a process switches to its copy. Setting it forces the switch whatever the tier compiled.
     - a number, at least 2
     - ``1024``
     - ``run``
   * - ``YAVS_SPEC_CAP``
     - Limit on copies, in percent of the shared optimised process code. ``0``: no limit.
     - a number
     - ``250``
     - ``run``
   * - ``YAVS_NO_PMEMO``
     - Turns the memo of pure function calls in the interpreter off.
     - any value
     - unset
     - ``run``
   * - ``YAVS_NO_NATIVE_IEEE``
     - Runs the ``ieee`` packages from their VHDL bodies only, without native fast paths.
     - any value
     - unset
     - ``elab``, ``run``
   * - ``YAVS_PACKS``
     - ``off`` gives every leaf a kernel signal of its own.
     - ``off``
     - on
     - ``elab``, ``run``
   * - ``YAVS_CELLS``
     - ``off`` lists the exact runs of changed elements for every waited part of a wide signal.
     - ``off``
     - on
     - ``run``
   * - ``YAVS_CODES``
     - ``all`` keeps the byte codes of every logic signal.
     - ``all``
     - unset
     - ``run``
   * - ``YAVS_LEAN_EVENTS``
     - ``off`` keeps the event history of every wide signal.
     - ``off``
     - on
     - ``run``
   * - ``YAVS_IDLE_SKIP``
     - ``off`` runs every activation of a clocked process; ``check`` also runs the ones the kernel would skip and fails when one does something else.
     - ``on``, ``off``, ``check``
     - ``on``
     - ``run``
   * - ``YAVS_POOL_ALL``
     - With ``--threads`` above 1, hands every batch of two or more processes to the thread pool, whatever it is worth.
     - ``1``
     - unset
     - ``run``

Compiled tiers: Cranelift
~~~~~~~~~~~~~~~~~~~~~~~~~

.. list-table::
   :header-rows: 1
   :widths: 22 38 16 10 14

   * - Variable
     - Purpose
     - Values
     - Default
     - Read by
   * - ``YAVS_CLIFT_ASM``
     - Prints the machine code of that function on standard error.
     - a function id
     - unset
     - ``run``
   * - ``YAVS_CLIFT_CLIF``
     - Prints the Cranelift IR of that function on standard error.
     - a function id, ``all``
     - unset
     - ``run``
   * - ``YAVS_CLIFT_CODEDIR``
     - Writes the machine code of each function into that directory.
     - a directory
     - unset
     - ``run``
   * - ``YAVS_CLIFT_DUMP``
     - Prints the IR of each function that is compiled.
     - any value
     - unset
     - ``run``
   * - ``YAVS_CLIFT_INLINE``
     - Largest body that is inlined. ``0``: none.
     - a number
     - ``150``
     - ``run``
   * - ``YAVS_CLIFT_LAYOUT_SLOTS``
     - ``--tier=auto`` also lays out the slots that cost frame memory.
     - ``1``
     - unset
     - ``run``
   * - ``YAVS_CLIFT_NATIVE``
     - ``0`` turns the native function variants off.
     - ``0``, ``1``
     - ``1``
     - ``run``
   * - ``YAVS_CLIFT_NATIVE_MAX``
     - Most native variants that are made.
     - a number
     - no limit
     - ``run``
   * - ``YAVS_CLIFT_NO_SCAN``
     - Search loops run without the scan helper.
     - any value
     - unset
     - ``run``
   * - ``YAVS_CLIFT_NTAB``
     - Every call goes through the table of native functions.
     - ``1``
     - ``0``
     - ``run``
   * - ``YAVS_CLIFT_OPT``
     - The Cranelift optimisation level.
     - ``speed``, ``none``
     - ``speed``
     - ``run``
   * - ``YAVS_CLIFT_PERFMAP``
     - Writes ``/tmp/perf-<pid>.map`` for ``perf report``.
     - ``1``
     - ``0``
     - ``run``
   * - ``YAVS_CLIFT_RA``
     - The register allocator.
     - ``backtracking``, ``single_pass``
     - ``backtracking``
     - ``run``
   * - ``YAVS_CLIFT_TRACE``
     - Prints a line for each materialised frame.
     - any value
     - unset
     - ``run``
   * - ``YAVS_CLIFT_VERIFY``
     - The Cranelift verifier.
     - ``0``, ``1``
     - ``1`` in a debug build, else ``0``
     - ``run``

Compiled tiers: when to compile
~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~

.. list-table::
   :header-rows: 1
   :widths: 22 38 16 10 14

   * - Variable
     - Purpose
     - Values
     - Default
     - Read by
   * - ``YAVS_AUTO_FORCE``
     - Compiles every function of that name with at least ``min`` instructions and an id of at least ``first``.
     - ``name@min#first,...``
     - unset
     - ``run``
   * - ``YAVS_AUTO_IPUS``
     - Instructions per microsecond the policy assumes.
     - a positive number
     - ``16000``
     - ``run``
   * - ``YAVS_AUTO_MODEL``
     - Overrides parts of the policy's cost model.
     - ``KEY=VALUE/...``
     - unset
     - ``run``
   * - ``YAVS_AUTO_PRICE``
     - ``model`` counts no function as cached.
     - ``model``
     - unset
     - ``run``
   * - ``YAVS_AUTO_PROFILE``
     - ``off`` does not load the profile of the last run.
     - ``off``
     - on
     - ``run``
   * - ``YAVS_AUTO_TRACE``
     - Prints the timeline of the compile thread on standard error.
     - any value
     - unset
     - ``run``
   * - ``YAVS_FIRST_ONLY``
     - The first-level generator is the only one, with no later recompile.
     - any value
     - unset
     - ``run``
   * - ``YAVS_FIRST_TUNE``
     - Overrides the first-level thresholds for a measurement.
     - ``KEY=N,...``
     - unset
     - ``run``
   * - ``YAVS_MIXED``
     - A build with both generators: LLVM compiles functions of even id, Cranelift the others.
     - any value
     - unset
     - ``run``

Compiled tiers: LLVM (the ``llvm`` feature of a source build)
~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~

.. list-table::
   :header-rows: 1
   :widths: 22 38 16 10 14

   * - Variable
     - Purpose
     - Values
     - Default
     - Read by
   * - ``YAVS_LLVM_ABLATE``
     - The interpreter steps these instruction kinds instead of compiling them.
     - ``Kind,Kind``
     - unset
     - ``run``
   * - ``YAVS_LLVM_ARENA``
     - ``0`` uses LLVM's own linker in place of the code arena.
     - ``0``
     - on
     - ``run``
   * - ``YAVS_LLVM_ARGS``
     - Replaces the default LLVM options.
     - LLVM options
     - built in
     - ``run``
   * - ``YAVS_LLVM_CODEMODEL``
     - ``large`` keeps the default code model of the JIT.
     - ``large``
     - small
     - ``run``
   * - ``YAVS_LLVM_DUMP``
     - Writes LLVM IR before and after optimisation, and yavs IR, into that directory.
     - a directory
     - unset
     - ``run``
   * - ``YAVS_LLVM_ELOOP_HELPER``
     - Element loops use the helper call.
     - any value
     - unset
     - ``run``
   * - ``YAVS_LLVM_FAST_MIN``
     - Module size above which the fast code generator is used.
     - a number
     - ``150000``
     - ``run``
   * - ``YAVS_LLVM_HUGE``
     - ``0``: the code arena without huge pages.
     - ``0``
     - on
     - ``run``
   * - ``YAVS_LLVM_KINDLOC``
     - Debug lines give the IR kind of each instruction; the value is the file for the kind names.
     - a file
     - unset
     - ``run``
   * - ``YAVS_LLVM_KINDS``
     - Prints the LLVM instructions made per IR kind.
     - any value
     - unset
     - ``run``
   * - ``YAVS_LLVM_NO_ELOOP``
     - Leaves the element loop hooks out.
     - any value
     - unset
     - ``run``
   * - ``YAVS_LLVM_OBJ``
     - Writes the object files into that directory.
     - a directory
     - unset
     - ``run``
   * - ``YAVS_LLVM_ONLY``
     - ``--tier=llvm`` compiles only these function ids.
     - ``12,34``
     - all
     - ``run``
   * - ``YAVS_LLVM_PCLOC``
     - With ``YAVS_LLVM_KINDLOC``: debug lines give the IR pc.
     - any value
     - unset
     - ``run``
   * - ``YAVS_LLVM_PERFMAP``
     - Writes ``/tmp/perf-<pid>.map`` for ``perf report``.
     - any value
     - unset
     - ``run``
   * - ``YAVS_PERFMAP``
     - The same, from the JIT's symbol lookups.
     - any value
     - unset
     - ``run``
   * - ``YAVS_LLVM_PIPELINE``
     - Replaces the default pass pipeline.
     - a pass pipeline
     - built in
     - ``run``
   * - ``YAVS_LLVM_PREKEY_CHECK``
     - Generates the code even on a cache hit and fails when it differs.
     - not ``0``
     - unset
     - ``run``
   * - ``YAVS_LLVM_TIMES``
     - Prints the times of each compile.
     - any value
     - unset
     - ``run``
   * - ``LLVM_SYS_211_PREFIX``
     - Where the build finds LLVM.
     - a directory
     - the usual places
     - the build

Tests and builds
~~~~~~~~~~~~~~~~

.. list-table::
   :header-rows: 1
   :widths: 22 38 16 10 14

   * - Variable
     - Purpose
     - Values
     - Default
     - Read by
   * - ``YAVS_BLESS``
     - Tests rewrite their expected files and the generated pages instead of comparing.
     - any value
     - unset
     - tests
   * - ``YAVS_CORPUS``
     - The directory of external corpora for the corpus tests.
     - a directory
     - per test
     - tests
   * - ``YAVS_SEM_XFAIL_WRITE``
     - Rewrites the expected-failure list of the semantic tests.
     - any value
     - unset
     - tests
   * - ``YAVS_SIM1_CMD``
     - The command line that runs a reference simulator, for an ignored test.
     - a command
     - unset
     - tests
   * - ``YAVS_PERF_PRINT``
     - The work-counter tests print every counter.
     - any value
     - unset
     - tests
   * - ``YAVS_IEEE_KEEP``
     - Directory where the ``ieee`` comparison tests keep their testbenches.
     - a directory
     - unset
     - tests
   * - ``YAVS_IEEE_SCALE``
     - Divides the iterations of the ``ieee`` comparison tests.
     - a positive number
     - ``1``
     - tests
   * - ``YAVS_IEEE_TIER``
     - The tier of the ``ieee`` comparison tests.
     - a tier
     - the build's default
     - tests
   * - ``YAVS_LIB_TEST_CHILD``
     - Set by the library tests for their child processes.
     - 
     - unset
     - tests
   * - ``YAVS_LIB_TEST_REWRITER``
     - Set by the library tests for their child processes.
     - 
     - unset
     - tests
   * - ``YAVS_HOST``
     - The host triple, from the build script.
     - 
     - set by the build
     - tests
   * - ``YAVS_TARGET``
     - The target triple, from the build script.
     - 
     - set by the build
     - tests
   * - ``YAVS_VHPI_DEF``
     - The Windows definition file of the VHPI exports, from the build script.
     - 
     - set by the build
     - tests
   * - ``YAVS_VHPI_HOST``
     - The host triple, from the build script.
     - 
     - set by the build
     - tests
   * - ``YAVS_VHPI_IMPORT_LIB``
     - The Windows import library of the VHPI exports, from the build script.
     - 
     - set by the build
     - tests
   * - ``YAVS_VHPI_TARGET``
     - The target triple, from the build script.
     - 
     - set by the build
     - tests
   * - ``YAVS_SRC_HASH``
     - A hash of all sources, from the build script; part of every cache key.
     - 
     - set by the build
     - the build

See also
--------

* :doc:`cli-reference`: every option, some of which these variables mirror.
* :doc:`on-disk`: the stores ``YAVS_STD_CACHE`` and the compile cache use.
* :doc:`threads`: the token server behind ``YAVS_TOKENS``.
* :doc:`vunit`: ``VUNIT_SIMULATOR`` and ``VUNIT_YAVS_PATH`` in a run script.

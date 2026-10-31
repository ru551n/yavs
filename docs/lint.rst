Static analysis
===============

``yavs lint`` reports what can be proved wrong about a design before it is simulated. A default run
reports definite errors only: the program cannot do what it says, whatever anyone intended. It
reports what it can derive and nothing it would have to guess, so an empty report means nothing
provable was found, and a line in it is something to correct.

.. code-block:: console

   $ yavs lint src/fifo.vhd
   error[lint_771]: `depth` can reach the end of its body without returning a value, which is an error when it does
     --> src/fifo.vhd:21:12
      |
   21 |   function depth (w : natural) return natural is
      |            ^

:doc:`lint-rules` lists every rule; ``yavs lint --explain lint_771`` prints one and
``yavs lint --list-rules`` prints them with what this configuration runs.

What to check
-------------

Check what is already in a library, or name files and directories:

.. code-block:: console

   $ yavs analyze --work=mylib:out/mylib src/*.vhd
   $ yavs lint --work=mylib:out/mylib              # every file of the library
   $ yavs lint --work=mylib:out/mylib src/fifo.vhd # one file, what it needs from the library
   $ yavs lint src/pkg.vhd src/fifo.vhd            # no library: in this order, as library work
   $ yavs lint --recursive src                     # a directory, sorted by name

Nothing is written into a library. Files a library records are read in its order, so analyze first
when a directory does not sort into dependency order. Give the ``--define``, ``--psl`` and
``--relaxed`` the files were analyzed with, as for ``yavs run``. A file that fails analysis prints
its errors and is skipped; the others are still checked.

The exit status is 0 without a finding of severity error, 1 with one, and 2 for a usage error, a
bad configuration or a file that fails analysis.

How sure a rule is
------------------

.. list-table::
   :header-rows: 1
   :widths: 20 60 20

   * - Class
     - What a finding means
     - Default
   * - ``definite``
     - The program cannot do what it says.
     - on
   * - ``advisory``
     - An exact fact, whose significance depends on what you meant.
     - off
   * - ``experimental``
     - Inferred design intent.
     - off
   * - ``policy``
     - A house convention.
     - off

Configuration
-------------

``yavs-lint.yaml`` in the working directory or an ancestor, or the file ``--config`` names, sets
which rules run and how a finding counts. A rule's own setting wins over its class's, which wins over
``global``:

.. code-block:: yaml

   rule:
     global:
       severity: warning      # a finding is reported, and does not fail the run
     group:
       advisory:
         disable: false       # one class on
     lint_772:
       disable: true          # one rule off
       severity: error

``lint_602`` takes ``suffixes`` and ``lint_603`` takes ``prefixes``, lists in which ``*`` and ``?``
are allowed. An unknown key or rule is an error naming it, with status 2. ``-Wno-RULE``, ``-Werror`` and
``-Werror=RULE`` do the same for one run; ``-Werror`` makes a finding of severity warning an error.

To leave files out, list them under ``exclude``: paths or globs, relative to the configuration file,
of files or directories. A pattern that starts with ``**`` also matches files outside that directory.
An excluded file is still read when another file needs it:

.. code-block:: yaml

   exclude:
     - generated                # a directory
     - 'src/**/old_*.vhd'       # `*` and `?` stop at `/`, `**` does not
     - '**/vunit/vhdl/**'       # a test runner's own libraries, wherever they are installed

Waivers
-------

A waiver accepts a finding a project has decided about, with the reason. They live in
``yavs-lint-waivers.yaml`` next to the configuration, or the file ``--waivers`` names:

.. code-block:: yaml

   waivers:
     - rule: lint_772
       files: 'src/legacy/**/*.vhd'  # `*` and `?` stop at `/`, `**` does not
       lines: [12, 45]               # omit to waive the rule in the whole file
       reason: kept until the block is replaced

``rule: '*'`` covers every rule, and a bare file name matches that file in any directory. A waived
finding is in no report and changes no exit status.

Reports
-------

Findings are ordinary diagnostics on standard error, so ``--message-format=short`` and ``json``
work as for every command. For a pipeline, write them to files as well:

.. list-table::
   :header-rows: 1
   :widths: 30 70

   * - Option
     - Format
   * - ``--json=FILE``
     - Files with their findings: rule, line, severity, message.
   * - ``--sarif=FILE``
     - SARIF 2.1.0, for code scanning. The class of a rule is its tag.
   * - ``--junit=FILE``
     - JUnit, one test case per source file.
   * - ``--quality-report=FILE``
     - Code Climate, for GitLab code quality.
   * - ``--sonarqube=FILE``
     - SonarQube generic issues. A finding of a definite rule is a bug.

Design facts
------------

``--facts=FILE`` writes what the analysis knows about each architecture as JSON, whichever rules are
on, with a ``schema_version``:

.. code-block:: json

   {
     "schema_version": 1,
     "architectures": [{
       "unit": "fifo(rtl)",
       "file": "src/fifo.vhd",
       "state_machines": [{
         "type": "state_t", "signals": ["state"], "states": ["idle", "run"],
         "transitions": [{"from": "idle", "to": "run"}], "reset_state": "idle"
       }],
       "clock_domains": [{"clock": "clk", "registers": ["state"]}],
       "reset_domains": [{"reset": "rst", "registers": ["state"]}],
       "crossings": [{"kind": "clock", "signal": "flag", "from": "clk_a", "to": "clk_b", "line": 42}]
     }]
   }

With VUnit
----------

``python run.py --compile --yavs-lint`` checks every library the run compiled, after compiling, and
fails the run with a finding of severity error (:doc:`vunit-yavs`). It reads ``yavs-lint.yaml`` from
the directory you run in or an ancestor, so ``exclude`` applies.

In VS Code
----------

The extension contributes the problem matcher ``$yavs``, so a task shows findings in the Problems
view and in the editor:

.. code-block:: json

   {
     "label": "yavs lint",
     "type": "shell",
     "command": "yavs lint --message-format=short --recursive src",
     "problemMatcher": "$yavs"
   }

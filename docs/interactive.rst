Interactive mode
================

``yavs run --interactive`` (needs a release newer than 0.11.0) runs a simulation from a terminal prompt, without an editor. It
elaborates the design, stops at time 0 before any process has run, and waits at ``yavs>``:

.. code-block:: text

    $ yavs run --work=work:build/work tb --interactive
    yavs> break tb.vhd:24
    breakpoint 1: /home/me/tb.vhd:24
    yavs> run 10 ns
    step at 10 ns, delta 2
    yavs> examine cnt
    4
    yavs> force cnt 5
    :tb:cnt = 5
    yavs> run
    breakpoint at 30 ns, delta 0, /home/me/tb.vhd:24
    yavs> restart
    yavs> quit

The prompt has line editing and history. Ctrl-C pauses a running simulation and returns to the
prompt; at the prompt it drops the line. Ctrl-D or the end of piped input is ``quit``.

.. contents:: On this page
   :local:

Commands
--------

``run``
    Runs to the end or the next breakpoint. ``run 100 ns`` runs for that time from now.
    ``run until cnt = 4`` runs until a change of a signal in the condition makes it true.
``step``, ``next``
    To the next statement: ``step`` enters calls, ``next`` does not. ``step delta`` and
    ``step timestep`` run to the end of the delta cycle or the time step.
``stop``
    Pauses a running simulation, from a client that can send it then (the terminal has Ctrl-C).
``examine NAME``, ``print NAME``
    The value of a local or a signal, or of a field, element or slice of one
    (``examine v(3 downto 0)``). Between cycles, simple names are looked up at the top level;
    inside a process, where that process sees them. A bare name also works.
``force NAME VALUE``, ``deposit NAME VALUE``, ``release NAME``
    Change a signal or a part of one, from the next cycle. Values are VHDL literals. ``-out`` at
    the end of ``force`` or ``release`` acts on the driving value instead of the effective one.
``where``
    The time, delta, process and source line.
``break``
    ``break tb.vhd:24 [if COND]`` stops at a line; ``break 24`` in the file of the current stop.
    ``break cnt [= 4] [if COND]`` stops at a change of a signal, or a change to a value.
    ``break list`` and ``break delete N``, ``break delete all``.
``restart``, ``reset``
    Back to time 0 with the same design and breakpoints; nothing is compiled. A wave file is
    written again from the start. Runs with ``--vhpi`` or ``--load`` refuse it, as the state of
    the library cannot be reset: start the run again.
``wave add NAME...``, ``wave cursor [TIME]``
    With ``--surfer`` (:doc:`waveforms`): show signals or scopes in the viewer; move its cursor, by default to now.
``source FILE``
    Runs the commands of a file. The first command that fails ends it.
``help``, ``quit [STATUS]``
    The exit status is ``STATUS``; without one, that of the run if it has ended, else 0.

Several commands go on a line with ``;``. ``#`` at the start of a command and ``--`` anywhere
start a comment.

Reports and text output go to standard output as in a batch run, and so do the results of
commands. Notices of stops and errors go to standard error.

Scripts
-------

``--do`` runs commands without a terminal. It can be repeated:

.. code-block:: sh

    yavs run --work=work:build/work tb --do "break tb.vhd:24; run; examine cnt" --do "source more.do"

Without ``--interactive`` the run exits at the end of the script with its status. A command that
fails ends the script with status 1. With ``--interactive``, the prompt follows the script, and a
failed command only prints its error. ``--do "run; quit"`` prints what a plain run prints and exits
with the same status.

In VS Code
----------

The same commands work in the Debug Console of a ``yavs`` session (:doc:`debugging`): type
``run 10 ns``, ``break list`` or ``restart``. A plain name still shows its value. ``restart`` stops
at time 0 and keeps the breakpoints of the editor. Unlike at the prompt, the session ends with the
run. Breakpoints made with ``break`` belong to the session only: the editor's Breakpoints list does not
show them, or their deletion. Watch expressions and hovers take plain names; only ``examine`` takes a part of
a signal (``slv(3 downto 0)``, ``rec.field``).

With a test runner
------------------

A test runner can add its own commands to the console: it registers a name and help text, and yavs
calls it when the command is typed. The handler runs in the test runner's process and can print
on the console, run console commands and load the design again. The protocol is in the
`command line contract <https://github.com/ru551n/yavs/blob/main/docs/cli-contract.md>`__
("Console hook").

The Python package ``vunit-yavs`` uses it: ``python run.py lib.tb.test_name --yavs-interactive``
starts one test at the prompt, and ``vunit_help`` lists what it adds. See its README.

Limits
------

The console is a debug session: the interpreter on one thread (``--threads`` and ``--tier`` have no
effect), no ``--cov`` or ``--profile``, and breakpoint conditions cannot call functions.
``run until`` watches signals, so a condition on variables alone is refused.

See also
--------

* :doc:`debugging`: the same session in VS Code.
* :doc:`vunit`: the console with a test of a run script.
* :doc:`waveforms`: ``--surfer`` and the ``wave`` commands.

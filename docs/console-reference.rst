Console command reference
=========================

Every command of the interactive console: the prompt of ``yavs run --interactive`` (:doc:`interactive`), the
scripts of ``--do`` and ``source``, and the Debug Console of a ``yavs`` session in an editor
(:doc:`debugging`). The commands are the same in all three. For a first walk through them, see
:doc:`interactive` and :doc:`debugger-tutorial`; this page is the complete list.

The synopsis of every command comes from the table the console itself is built from, so the
page cannot name a command that does not exist, and a command cannot be added without an entry
here. ``help`` shows the same synopses at the prompt, and ``help COMMAND`` the synopsis and
meaning of one command. Every example below was run, and the output shown is what it printed.
The examples use ``tb.vhd`` and ``types.vhd`` of ``docs/examples/debugger`` in the source tree,
analyzed with ``yavs analyze --std=2008 --work=work:lib``, with the design in ``/work``.

.. contents:: On this page
   :local:
   :depth: 2

How a line is read
------------------

Lines and commands
~~~~~~~~~~~~~~~~~~

A line holds one or more commands separated by ``;``. Empty commands are skipped, so ``run 2 ns;;
where`` is two commands. A semicolon inside a string (``"a;b"``) or a character literal
(``';'``) does not separate.

A comment starts with ``#`` at the start of a command, or with ``--``; it ends the line, so a
``;`` after it does not start another command. A ``#`` in the middle of a command is part of the
command. The one exception for ``--``: after a space and right before a letter inside a command
it is a flag (``vunit_load --stop-time=30ns``), not a comment.

.. example:

.. code-block:: text

    yavs> run 2 ns;;  examine clk
    step at 2 ns, delta 0
    '0'
    yavs> # a comment
    yavs> examine cnt -- a comment; examine q
    "0000"
    yavs> examine clk; examine q
    '0'
    'U'

The first word is the command. It is matched exactly, in lower case; ``print`` and ``reset`` are
other names for ``examine`` and ``restart``. A line whose first word is no command is taken as ``examine`` of the whole line, so typing a name
shows its value. When that fails too, the error names the word and suggests the closest command:

.. example:

.. code-block:: text

    yavs> cnt
    "0000"
    yavs> exmine cnt
    error: unknown command or signal `exmine`: a command with a similar name exists: `examine`
    yavs> frobnicate
    error: unknown command or signal `frobnicate`: see `help`

Results go to standard output, as the output of a batch run does. Notices of stops (``step at
...``, ``breakpoint at ...``) and errors (``error: ...``) go to standard error. With ``--do`` and a
failed command, ``error:`` is the last line before the run ends (see `Scripts`_).

Names
~~~~~

A name in a command is a signal, a variable, a part of one, or a path:

* A simple name (``cnt``) is looked up case-insensitively. In a process that has stopped, the
  locals of the frame come first, then the variables of the process, then the signals the
  process sees (those of its architecture and its ports). Inside a method of a protected type
  the variables of the object are in scope too. Between cycles (after ``run 12 ns``, no
  process stopped) a simple name is looked up at the top level.
* A generic of the instance (``N``) is a name in an expression, after the signals: ``examine N``,
  ``break cnt if cnt = N``. Arrays of characters, enumerations, numbers and times are the generics
  it reads; between cycles the generics of the top level.
* A path names a signal anywhere. ``tb.u_reg.q``, ``:tb:u_reg:q`` and ``TB.U_REG.Q`` are the same
  signal; the dotted form is what ``find`` and the messages print. Case does not matter, and a
  leading ``:`` starts a path of colons. A path may also start from where the name is looked up:
  ``u_reg.q`` from a process of ``tb``, or between cycles.
* A variable of a process has the path ``PROCESS.NAME``: ``tb.counter.total``. In a process without
  a label, which tables call ``tb.process (tb.vhd:34)``, ``PROCESS`` is an internal name that fits a
  path: ``tb._p0.total``, as ``find variables`` lists it. ``find variables``
  lists them. At a stop in that process ``total`` is enough.
* A shared variable is a name like a signal: seen from the process stopped in, or by its path
  (``tb.tally``). One of a package has the path ``LIBRARY.PACKAGE.NAME``; ``PACKAGE.NAME`` is
  enough, and so is ``NAME`` if only one package has it. A protected object shows as ``protected``,
  and its variables are parts of it: ``tally.total``. Calling a method is not a name.
* A part of an object follows the name: a field (``rec.x``), an element (``v(3)``), a slice
  (``v(3 downto 0)``, ``v(0 to 1)``), through an access value (``p.next_node.value``). Spaces
  inside the parentheses are fine.

A name that matches nothing says so and suggests the closest names of the kind the command
takes: signals and variables for ``examine``, signals for ``drivers`` and ``force``, fields of a
record, subcommands, commands.

.. example:

.. code-block:: text

    yavs> examine tb.cnt
    "0000"
    yavs> examine :tb:cnt
    "0000"
    yavs> examine u_reg.q
    'U'
    yavs> examine u_reg.qq
    error: no variable or signal `u_reg.qq` here: a signal with a similar name exists: `tb.u_reg.q`
    yavs> examine TB.Cnt
    "0000"
    yavs> examine cnt(3 downto 2)
    "00"
    yavs> examine ctn
    error: no variable or signal `ctn` here: a signal or variable with a similar name exists: `cnt`
    yavs> examine tb.u_reg.qq
    error: no variable or signal `tb.u_reg.qq` here: a signal with a similar name exists: `tb.u_reg.q`

Values
~~~~~~

``force``, ``deposit`` and the value of ``break SIGNAL = VALUE`` are VHDL literals of the type of
the target, written as in VHDL source, or with the quotes left out for a bit-like target (below):

* An integer: ``42``, ``-3``, ``16#FF#``, ``1_000``. It must be in the range of the subtype.
* A real: ``0.5``, ``1.0e-3``, ``2``.
* An enumeration: the literal (``busy``, ``true``, ``'1'``, ``'Z'``), not case sensitive
  for identifiers.
* A physical value, ``time`` among them: a number and a unit, ``5 ns``, ``2.5 us``. Without a
  unit a number is an error. It is shown in the largest unit that holds it whole (``10 ns``,
  ``1500 ps``). A fraction that is no whole number of the primary unit (``0.5 fs``) is an error.
* A string or bit string for an array of characters: ``"0101"``, ``x"F0"``, ``b"1010"``,
  ``o"17"``, ``8ux"F"`` (sized, as VHDL-2008). The length must be the length of the target.
* An aggregate for a record or an array: positional (``(7, true, '1')``), named (``(x => 7,
  ok => true, tag => '1')``), with an index range (``(7 downto 4 => '1', others => '0')``) and
  with ``others``.
* ``null`` for an access value.

A target of a bit-like type (``bit``, ``std_ulogic``, ``std_logic``, an enumeration of character literals with ``'0'`` and
``'1'``) or a vector of one needs no quotes: ``1`` is ``'1'``, ``0101`` is ``"0101"``. What is already a literal of the
target keeps its meaning, so an integer's ``1`` stays 1. Nothing is guessed elsewhere: ``5`` is not a value of an
``unsigned``, and a ``string`` keeps its quotes.

The value of a part is the type of the part: ``force bus8(1 downto 0) "11"``. ``examine`` prints
values in the same forms, as ``"0101"``, ``'1'``, ``idle``, ``(x => 7, ok => TRUE, tag =>
'1')``. An array of more than 256 elements ends in ``...``.

Time
~~~~

A time is a number and a unit, with or without a space: ``10 ns``, ``10ns``, ``1.5 us``,
``250 ps``. The units are ``fs``, ``ps``, ``ns``, ``us``, ``ms``, ``sec``, ``min`` and ``hr``, in
any case. The number is not negative. A time without a unit, such as ``run 10``, is an error, and so is a fraction of
the resolution (``run 0.5 fs``: the resolution is 1 fs). ``run`` takes a
time relative to now; ``examine NAME @ TIME`` and ``wave cursor`` take an absolute one.
Times are printed in the largest unit that shows them whole (``12 ns``, ``1510 ps``).

Conditions
~~~~~~~~~~

``break ... if CONDITION``, ``run until CONDITION`` and ``examine`` take a VHDL expression over
the names the stop sees. ``if`` ends the place of a ``break`` and starts the condition.

* Operators: ``and or nand nor xor xnor not``, ``= /= < <= > >=``, ``+ - * / mod rem abs``, a
  sign, and ``??``. Not supported: ``&``, ``**``, shifts and rotates, the matching operators
  ``?=``, reduction operators; they are refused when the command is made, naming the operator.
* Literals as in `Values`_, names as in `Names`_ with fields, elements, slices and access
  values. A bare literal is read in the type of the other operand (``cnt = "0010"``,
  ``q = '1'``).
* ``unsigned`` and ``signed`` signals compare with numbers and with each other (``cnt > 3``). A
  time is compared and added with times, and scaled by an integer.
* Calls of functions visible from the process: those of the design and of packages such as
  ``numeric_std`` and ``std_logic_1164`` (``to_integer(unsigned(d)) > 40``, ``resize(cnt, 8) + 1``,
  ``f(n) = 3``), with positional arguments; the overload is chosen by the types of the arguments.
  A function without parameters is its bare name (``edges > 2``), and ``now`` is the time of the
  stop. Type conversions (``unsigned(d)``, ``integer(x)``). An operator that a type declares calls
  that function: ``cnt + 1 = 4`` on an ``unsigned``, ``(clk and en) = '1'`` on ``std_logic``.
* Pure and impure functions are called if they only read: an impure function may read the
  signals of its architecture, block or entity (their values at the stop) and ``now``. Before a
  function is called, its code and that of everything it calls are checked: one that may write
  (assign a signal, call a method of a protected object, use a shared variable or a file, end the
  simulation with ``std.env.stop``, call foreign code) is refused, naming what it does and where
  (``tick may write, so a condition does not call it: it calls a method of a protected object
  (tb.vhd:235)``). A called function runs apart from the simulation on copies of what it reads, so
  it cannot change the simulation, and a report it makes is neither printed nor counted: a run
  with conditions prints and exits as one without them.
* Also refused, with the reason: functions with a signal parameter (write ``clk'event and clk =
  '1'`` for ``rising_edge(clk)``) or an ``out`` or ``inout`` parameter, functions that read an
  attribute of a signal or a signal of a package, functions that use a shared variable of a type
  that is not protected (reading and writing one are not told apart), foreign functions, functions declared in a process or a subprogram, and functions that
  read a constant whose value is computed only when the simulation starts (one of a package, or
  one of the design that elaboration did not need). A function that never returns holds the
  stop.
* The type of a field of a record and of a variable of a subprogram is not known at a stop: an
  operator of ``numeric_std`` on one needs a conversion (``unsigned(y) + 1``); comparisons with
  numbers still work.
* Attributes of a signal or part of one: ``'event``, ``'active``, ``'last_value``,
  ``'last_event``, ``'last_active`` (between cycles, of the cycle just run); of an array:
  ``'length``, ``'left``, ``'right``, ``'high``, ``'low``, ``'ascending`` (integer indices). Other
  attributes, attributes of types and attributes with an argument are refused, naming them.

The condition is checked when the command is made, with the text and the help it would have
at a stop. A syntax error or an unsupported operator is refused, and so are an unknown name, a
literal that does not fit the type of the signal it is compared with, operands of different types
and a result that is not boolean. What only a stop knows cannot be checked: the value of a
variable of a process, so its type (``total > 2`` is not checked against ``total``'s type), and
what depends on a value, such as a division by zero, an index or an access value that is
``null``. The reply of the command names the variables that were not checked. They are
evaluated when the place is reached, and a condition that fails to evaluate then stops the run
there as an ``exception`` with the reason. In ``examine`` the result is printed whatever its
type.

.. example:

.. code-block:: text

    yavs> break tb.vhd:43 if total > 2
    breakpoint 1: /work/tb.vhd:43
      not checked until the condition is evaluated at a stop: `total`
    yavs> break tb.vhd:43 if total > 2 and cnt = "0010"
    breakpoint 2: /work/tb.vhd:43
      not checked until the condition is evaluated at a stop: `total`
    yavs> break tb.vhd:43 if total >
    error: condition `total >` does not parse: expected expression, found the end
    yavs> break tb.vhd:43 if cnt & cnt
    error: operator `&` is not supported in conditions
    yavs> break tb.vhd:43 if cn = "0010"
    error: condition `cn = "0010"`: no local or signal `cn` here: a signal or variable with a similar name exists: `cnt`
    yavs> break tb.vhd:43 if cnt
    error: condition `cnt`: the condition is not a boolean
    yavs> break tb.vhd:43 if cnt = '1'
    error: condition `cnt = '1'`: `'1'` is not a value of the other operand's type: expected 4 elements, such as 0101 or "0101", x".." or an aggregate
    yavs> run
    breakpoint at 25 ns, delta 0, /work/tb.vhd:43
    yavs> examine total
    3
    yavs> examine to_integer(cnt + 1) * cnt'length
    12
    yavs> examine clk'event and clk'last_value = '0'
    TRUE
    yavs> break tb.vhd:43 if rising_edge(clk)
    error: condition `rising_edge(clk)`: `rising_edge` takes a signal parameter, which a condition cannot pass (it reads values only): write the test with attributes, as in clk'event and clk = '1'
    yavs> examine total * 2 > 3
    TRUE

Keys at the prompt
~~~~~~~~~~~~~~~~~~

The prompt of a terminal has line editing and history. The keys:

``Tab``
    Works as in a shell: one match completes and ends the word, several complete what they have
    in common, and a second Tab lists them and gives the line back. It offers what fits at the
    cursor. The first word is a command or a test runner's command, never a signal (a bare name
    is still examined when typed in full). After a command, what that command takes: signals and
    variables after ``examine``, ``describe``, ``drivers``, ``force`` and the like, down a dotted
    path and through fields after a ``.``; files after ``source`` and ``wave save``; source files
    and signals after ``break``; subcommands after ``wave``, ``find`` and ``step``; command names
    after ``help``; nothing for a time after ``run`` or a value after ``force``. The Debug Console
    of an editor uses the same table; there Tab takes what the list has in common, and with no
    list open opens it.

``Ctrl-C``
    While a command runs, pauses the simulation at the next point where it can stop; the prompt
    comes back and the run can go on with ``run``. At the prompt it drops the line being typed.
    Without a terminal (piped input), Ctrl-C while no command runs ends the process with
    status 130.

``Ctrl-D``
    At an empty prompt, or the end of piped input: ``quit``. The exit status is as for ``quit``
    without an argument.

Scripts
~~~~~~~

``--do COMMANDS`` runs the commands before the prompt, or instead of it. It can be repeated and
takes the same lines as the prompt (``;`` separates). ``source FILE`` runs the lines of a file in
the same way; a file may ``source`` others. In both:

* Without ``--interactive`` the run ends after the last command, and the status is that of the run
  as far as it went: 0, 1 after a reported error, or the one of ``quit STATUS``. A command that
  fails stops the script; the run ends at that point with status 1.
* With ``--interactive``, the prompt follows the script, and a failed command only prints its
  error.
* ``--do "run; quit"`` prints what a plain ``yavs run`` prints and ends with the same status.

.. example-shell:

.. code-block:: text

    $ yavs run --work=work:lib tb --do "run 10 ns; examine cnt; examine nosuch; examine clk"; echo "status $?"
    step at 10 ns, delta 0
    "0001"
    error: no variable or signal `nosuch` here
    status 1
    $ yavs run --work=work:lib tb --do "run 10 ns" --do "examine cnt"; echo "status $?"
    step at 10 ns, delta 0
    "0001"
    status 0
    $ yavs run --work=work:lib tb --do "run; quit"; echo "status $?"
    ** Error: 25000000fs+1: q is not high
        Process tb.check
    the simulation ended at 80 ns, delta 0
    status 1
    $ yavs run --work=work:lib tb --do "run 5 ns; quit 3"; echo "status $?"
    step at 5 ns, delta 2
    status 3
    $ echo "run 20 ns; examine cnt" | yavs run --work=work:lib tb --interactive; echo "status $?"
    step at 20 ns, delta 0
    "0010"
    status 0
    $ yavs run --work=work:lib tb --do "source setup.do"; echo "status $?"
    breakpoint 1: /work/tb.vhd:43
    breakpoint at 5 ns, delta 0, /work/tb.vhd:43
    1
    "0000"
    status 0

What a command needs
--------------------

The console is a debug session: the interpreter on one thread (``--threads`` and ``--tier`` have no
effect), and no ``--cov`` or ``--profile``. Every command works at a stop, which at the prompt is
always the case. Some need more:

.. include:: console-needs.inc

Besides these:

* ``break LINE`` without a file and a variable of a process read by its short name need the stop
  to be in a process, not between cycles.
* ``trace x`` and ``examine PROCESS.VARIABLE`` outside that process use a wave file when there is
  one: ``--wave=FILE.fst``. Without one, ``trace x`` says since when a value was last changed
  rather than since when it has been unknown, and ``examine`` says how to get the value.
* ``restart`` is refused in a run with ``--vhpi`` or ``--load``.
* ``examine NAME @ TIME`` and ``wave add`` of a protected object say that it is not recorded: waves
  hold the other shared variables, whose ``examine`` at a stop and whose history share one name.
  ``force``, ``deposit``, ``release`` and ``drivers`` take signals only.
* The ``vunit_*`` commands, and any others a test runner registers, exist only when the test
  runner started the session (`Test runner commands`_). ``help`` lists them when they do.

The commands
------------

Each command starts with its synopsis (the forms it takes, as ``help`` prints them) and the
one-line meaning, both generated from the console's own table. ``UPPER CASE`` words are
arguments, ``[ ]`` optional, ``|`` a choice, and ``...`` that the last argument repeats.

Run and step
~~~~~~~~~~~~

.. _console-run:

run
^^^

.. console: run

.. code-block:: text

    run
    run TIME
    run until CONDITION

To the end or a breakpoint, for TIME, or until a change of a signal in the condition makes it true.

.. console-end

``run`` with no argument goes to the end of the simulation or to the next breakpoint. When the
simulation stops it says why on standard error: ``breakpoint at 5 ns, delta 0, FILE:LINE``,
``data breakpoint at ...``, ``assertion of severity error at ...`` and the like. At the end it
prints ``the simulation ended at TIME, delta N``; from then on ``run`` only repeats that the
simulation has ended (``restart`` runs it again, ``quit`` leaves).

``run TIME`` runs for ``TIME`` from now (`Time`_), and stops there with ``step at TIME, delta N``,
or earlier at a breakpoint or the end.

``run until CONDITION`` runs until a change of a signal named in ``CONDITION`` (`Conditions`_)
makes it true, and stops there, saying which signal changed (``data breakpoint at 15 ns, delta
1: tb.cnt = "0010"``). Breakpoints still stop it. The condition is checked at once, as for
``break`` (`Conditions`_); what only the stop knows shows at the first change as a stop with the
reason (``exception at 15 ns, delta 1: ...``). A change is read between cycles, where no
process's variables are: a variable in the condition is an unknown name. A condition that names no signal (``run until 1 = 1``) is refused: it would have nothing to watch.

.. example:

.. code-block:: text

    yavs> run 10
    error: a time needs a unit: `10 ns`
    yavs> run 0.5 fs
    error: `0.5 fs` is not a time: physical value "0.5 fs" is not a whole number of fs: the resolution is 1 fs
    yavs> run until nosuch = 1
    error: condition `nosuch = 1`: no local or signal `nosuch` here
    yavs> run 12 ns
    step at 12 ns, delta 0
    yavs> run until cnt = "0010"
    data breakpoint at 15 ns, delta 1: tb.cnt = "0010"
    yavs> run
    ** Error: 25000000fs+1: q is not high
        Process tb.check
    the simulation ended at 80 ns, delta 0
    yavs> run
    the simulation has ended: `restart` runs it again, `quit` leaves

.. _console-step:

step
^^^^

.. console: step

.. code-block:: text

    step
    step delta
    step timestep

To the next statement, entering calls; or to the end of the delta or time step.

.. console-end

``step`` runs to the next statement, entering a subprogram called from the statement. At time 0
it enters the first process. ``step delta`` runs the rest of the delta cycle, and ``step
timestep`` the rest of the time step. All of them print ``step at TIME, delta N`` and, when
stopped in a process, its file and line. After the end of the simulation they fail with the
message of ``run``.

.. _console-next:

next
^^^^

.. console: next

.. code-block:: text

    next

To the next statement, over calls.

.. console-end

``next`` is ``step`` that does not enter calls: the statement of the call runs to its end.

.. example: top=types_tb

.. code-block:: text

    yavs> break types.vhd:50
    breakpoint 1: /work/types.vhd:50
    yavs> run
    breakpoint at 0 fs, delta 0, /work/types.vhd:50
    yavs> step
    step at 0 fs, delta 0, /work/types.vhd:51
    yavs> step
    step at 0 fs, delta 0, /work/types.vhd:52
    yavs> next
    step at 0 fs, delta 0, /work/types.vhd:53
    yavs> where
    time 0 fs, delta 0
    types_tb.build, /work/types.vhd:53

.. _console-stop:

stop
^^^^

.. console: stop

.. code-block:: text

    stop

Pause a running simulation (Ctrl-C on a terminal).

.. console-end

The one command a running simulation takes: in an editor's Debug Console, ``stop`` while a
``run`` is under way pauses it at the next point it can. At the terminal prompt the simulation is
always stopped, a command is not read while one runs, and ``stop`` only says so; Ctrl-C
(`Keys at the prompt`_) does the pausing there.

.. example:

.. code-block:: text

    yavs> stop
    the simulation is stopped

Breakpoints
~~~~~~~~~~~

Breakpoints are numbered from 1 in the order they are made. The number is for ``break delete``
and ``break list``. They stay over ``restart``. A ``break`` that fails uses up no number.

.. _console-break:

break
^^^^^

.. console: break

.. code-block:: text

    break [FILE:]LINE [if CONDITION]
    break SIGNAL [= VALUE] [if CONDITION]
    break assert [note|warning|error|failure]
    break list
    break delete N
    break delete assert
    break delete all

Stop at a line, at a change of a signal, or at a report of this severity and worse; list or delete them.

.. console-end

``break`` alone is an error that points here: it needs a place.

.. _console-break-line:

break [FILE:]LINE
"""""""""""""""""

.. console: break line

.. code-block:: text

    break [FILE:]LINE [if CONDITION]

.. console-end

A breakpoint at a source line. ``FILE`` is a source file of the design by the name it has in
``break list`` or its file name alone; without ``FILE`` the line is in the file of the current
stop, which needs a stop in a process. A line with no statement moves to the next line that has
one, and the reply gives the line it moved to (``break tb.vhd:44`` answers ``breakpoint 2:
/work/tb.vhd:47``); past the last statement it is an error. ``break list`` shows the line the
breakpoint is at and, when it moved, the line asked for. The
run stops *before* the statement runs: ``breakpoint at TIME, delta N, FILE:LINE``. With ``if
CONDITION`` (`Conditions`_) it stops only when the condition holds there.

.. example:

.. code-block:: text

    yavs> break tb.vhd:43 if total > 1
    breakpoint 1: /work/tb.vhd:43
      not checked until the condition is evaluated at a stop: `total`
    yavs> break 43
    error: no source file here: write FILE:LINE
    yavs> break tb.vh:43
    error: `tb.vh` is not a source file of this design: a source file with a similar name exists: `tb.vhd`
    yavs> break tb.vhd:999
    error: no statement on or after this line
    yavs> run
    breakpoint at 15 ns, delta 0, /work/tb.vhd:43
    yavs> examine total
    2
    yavs> break 42
    breakpoint 2: /work/tb.vhd:42
    yavs> break tb.vhd:44
    breakpoint 3: /work/tb.vhd:47
    yavs> break tb.vhd:43 if nosuch = 1
    error: condition `nosuch = 1`: no local or signal `nosuch` here
    yavs> break list
    1 /work/tb.vhd:43 if total > 1
    2 /work/tb.vhd:42
    3 /work/tb.vhd:47 (asked for line 44)
    yavs> break delete 1
    1 deleted
    yavs> break delete 7
    error: no breakpoint 7: see `break list`

.. _console-break-signal:

break SIGNAL
""""""""""""

.. console: break signal

.. code-block:: text

    break SIGNAL [= VALUE] [if CONDITION]

.. console-end

A data breakpoint: stops at the first cycle in which ``SIGNAL`` (a name, `Names`_) changes, or,
with ``= VALUE`` (`Values`_), changes to that value. The stop says ``data breakpoint at TIME,
delta N: NAME = VALUE`` and is *after* the cycle in which the value changed, between processes.
With ``if CONDITION`` it also needs the condition, read after the change between cycles, so it
sees signals and not the variables of a process. A name that is no signal is an error with
the closest names (``break`` cannot watch a variable).

.. example:

.. code-block:: text

    yavs> break cnt
    breakpoint 1: tb.cnt
    yavs> break cnt = "0011"
    breakpoint 2: tb.cnt
    yavs> break q = '1' if cnt > 1
    breakpoint 3: tb.q
    yavs> break q if total > 1
    error: condition `total > 1`: no local or signal `total` here
    yavs> break list
    1 tb.cnt
    2 tb.cnt = "0011"
    3 tb.q = '1' if cnt > 1
    yavs> run
    data breakpoint at 5 ns, delta 1: tb.cnt = "0001"
    yavs> run
    data breakpoint at 15 ns, delta 1: tb.cnt = "0010"
    yavs> break cnt = 7
    error: `7` is not a value of `tb.cnt`, type UNSIGNED: expected 4 elements, such as 0101 or "0101", x".." or an aggregate
    yavs> break nosuch
    error: no signal `nosuch` here

.. _console-break-assert:

break assert
""""""""""""

.. console: break assert

.. code-block:: text

    break assert [note|warning|error|failure]

.. console-end

Stops at an ``assert`` that fails and at a ``report``, of the severity named and worse
(``note``, ``warning``, ``error``, ``failure``); with no severity, ``error``. The reply lists
the severities now watched. The run stops after the message is printed, in the statement that
made it, with the stack intact. The command replaces the earlier ``break assert``; ``break
delete assert`` removes it.

.. example:

.. code-block:: text

    yavs> break assert error
    stop at assertions and reports of severity error, failure
    yavs> run
    ** Error: 25000000fs+1: q is not high
        Process tb.check
    assertion of severity error at 25 ns, delta 1, /work/tb.vhd:53: q is not high
    yavs> where
    time 25 ns, delta 1
    tb.check, /work/tb.vhd:53
    yavs> break assert bogus
    error: `break assert bogus`: expected `note`, `warning`, `error` or `failure`
    yavs> break list
    assert: severity error, failure

.. _console-break-list:

break list
""""""""""

.. console: break list

.. code-block:: text

    break list

.. console-end

Prints the breakpoints, one per line: the number, the place (``FILE:LINE`` or the signal), and
``= VALUE`` and ``if CONDITION`` as given. The assertion stops come first as ``assert:
severity error, failure``. With none it says ``no breakpoints``.

.. _console-break-delete:

break delete
""""""""""""

.. console: break delete

.. code-block:: text

    break delete N
    break delete assert
    break delete all

.. console-end

``break delete N`` removes breakpoint ``N`` (``no breakpoint N`` if there is none),
``break delete assert`` the stops at assertions, ``break delete all`` everything. The reply
counts what was removed.

Inspect
~~~~~~~

.. _console-examine:

examine
^^^^^^^

.. console: examine

.. code-block:: text

    examine NAME
    examine NAME @ TIME

The value of a local or a signal, or a field, element or slice of one; with a time, from the wave file. Also written ``print``. ``examine NAME @ TIME`` needs a wave file: run with ``--wave=FILE.fst``.

.. console-end

Prints the value of ``NAME`` (`Names`_) as a literal (`Values`_). ``NAME`` may also be an
expression (`Conditions`_): ``examine cnt > 3``, ``examine to_integer(cnt) + 1``. Between cycles a
signal is read as it is now; in a process, a variable has the value before the statement the
process has stopped at. ``print`` is the same command.
A process variable read from outside its process, by ``PROCESS.NAME``, is read from the wave file.

.. example:

.. code-block:: text

    yavs> run 12 ns
    step at 12 ns, delta 0
    yavs> examine cnt
    "0001"
    yavs> cnt
    "0001"
    yavs> examine cnt(3 downto 2)
    "00"
    yavs> examine to_integer(cnt) + 1
    2
    yavs> print clk
    '0'
    yavs> examine
    error: `examine` needs a name
    yavs> examine ctn
    error: no variable or signal `ctn` here: a signal or variable with a similar name exists: `cnt`

Records, arrays, enumerations, access values and the variables of a protected object:

.. example: top=types_tb

.. code-block:: text

    yavs> break types.vhd:50
    breakpoint 1: /work/types.vhd:50
    yavs> run
    breakpoint at 0 fs, delta 0, /work/types.vhd:50
    yavs> step
    step at 0 fs, delta 0, /work/types.vhd:51
    yavs> step
    step at 0 fs, delta 0, /work/types.vhd:52
    yavs> step
    step at 0 fs, delta 0, /work/types.vhd:23
    yavs> step
    step at 0 fs, delta 0, /work/types.vhd:30
    yavs> examine tail.value
    2
    yavs> examine tail.next_node.value
    1
    yavs> examine tail.next_node.next_node
    null
    yavs> examine head.value + tail.value
    3
    yavs> examine pair
    (x => 0, ok => FALSE, tag => '0')
    yavs> examine pair.x
    0
    yavs> examine words(2)
    "00000000"
    yavs> examine state
    idle
    yavs> examine ratio
    0.5
    yavs> examine delay
    10 ns
    yavs> examine tail.nosuch
    error: no field `nosuch` in `tail.nosuch`

Inside a method of a protected type, the variables of the object are in scope:

.. example: top=types_tb

.. code-block:: text

    yavs> break types.vhd:30
    breakpoint 1: /work/types.vhd:30
    yavs> run
    breakpoint at 0 fs, delta 0, /work/types.vhd:30
    yavs> examine sum
    0
    yavs> examine n
    3
    yavs> examine sum + n
    3
    yavs> where
    time 0 fs, delta 0
    types_tb.build, /work/types.vhd:30

.. _console-examine-at:

examine NAME @ TIME
"""""""""""""""""""

.. console: examine at

.. code-block:: text

    examine NAME @ TIME

Needs a wave file: run with ``--wave=FILE.fst``.

.. console-end

Reads the value ``NAME`` had at ``TIME`` (`Time`_, absolute) from the wave file, so the run
needs ``--wave=FILE.fst``. A time after now is an error (``is still to come``). A name that the
wave options left out (``--wave-include``, ``--wave-exclude``, ``--wave-max-array``) is
``not recorded``; a composite is recorded element by element, and is printed as a table of its
elements. It is the way to read a process variable outside its process.

.. example: --wave=tb.fst

.. code-block:: text

    yavs> run 30 ns
    ** Error: 25000000fs+1: q is not high
        Process tb.check
    step at 30 ns, delta 0
    yavs> examine cnt @ 12 ns
    "0001"
    yavs> examine cnt @ 1 us
    error: 1 us is still to come: the simulation is at 30 ns
    yavs> examine cnt @ 12
    error: a time needs a unit: `12 ns`
    yavs> examine tb.counter.total @ 20 ns
    2
    yavs> examine cn @ 12 ns
    error: `cn` is not recorded in `tb.fst`: check `--wave-include`, `--wave-exclude` and `--wave-max-array`

.. _console-describe:

describe
^^^^^^^^

.. console: describe

.. code-block:: text

    describe NAME

Type, bounds, resolution function, mode and declaration of a signal or variable.

.. console-end

Prints a table for a signal or variable: kind (signal, port with its mode, variable, shared
variable, protected object), type, bounds, resolution, where it is declared and its value now. For
a variable of a process, use its path (``tb.counter.total``) or stop in the process. A protected
object lists its variables.

.. example:

.. code-block:: text

    yavs> describe cnt
    tb.cnt
      kind        signal
      type        UNSIGNED
      bounds      3 downto 0 (4 elements)
      resolution  resolved (per element)
      declared    tb.vhd:31
      value       "0000"
    yavs> describe u_reg.clk
    tb.u_reg.clk
      kind        port, mode in
      type        STD_LOGIC
      bounds      'U' to '-' (9 literals)
      resolution  resolved
      declared    tb.vhd:7
      value       '0'
    yavs> describe tb.counter.total
    tb.counter.total
      kind      variable of process tb.counter
      type      INTEGER
      declared  tb.vhd:39
    yavs> describe
    error: `describe` needs a name

.. _console-where:

where
^^^^^

.. console: where

.. code-block:: text

    where

Time, delta, process and source line.

.. console-end

Prints the time and delta, and the stopped process with its file and line. Between cycles it
says so instead of a process; before the first cycle, ``time 0 fs, delta 0``.

.. example:

.. code-block:: text

    yavs> where
    time 0 fs, delta 0
    between cycles
    yavs> run 7 ns
    step at 7 ns, delta 0
    yavs> where
    time 7 ns, delta 0
    between cycles
    yavs> break tb.vhd:43
    breakpoint 1: /work/tb.vhd:43
    yavs> run
    breakpoint at 15 ns, delta 0, /work/tb.vhd:43
    yavs> where
    time 15 ns, delta 0
    tb.counter, /work/tb.vhd:43

Change values
~~~~~~~~~~~~~

These change a signal; the change takes effect in the next cycle, not at once, so ``examine`` still
shows the old value until the simulation has run on (``step delta`` is enough). ``NAME`` is a signal or a part of one (`Names`_): a field, an element or a
slice, and ``VALUE`` is a literal of its type (`Values`_). The reply is the full path with the
value as it was understood: ``tb.cnt = "0101"``. Errors name the signal or the literal: no such
signal (with the closest names), a value of the wrong kind, a string of the wrong length, an
integer out of its range, an index out of range, a missing signal or value.

.. _console-force:

force
^^^^^

.. console: force

.. code-block:: text

    force NAME VALUE [-out]

Hold a signal at a value, from the next cycle; with `-out` the driving value, not what readers see.

.. console-end

``force`` holds the signal at the value until a ``release``, whatever its drivers do. Without
``-out`` it holds the value the readers see, after resolution. With ``-out`` (written last) it
holds the driving value, the one the signal's drivers contribute to the resolution. ``drivers``
shows both (``force: ...``, ``force out: ...``).

.. example:

.. code-block:: text

    yavs> run 22 ns
    step at 22 ns, delta 0
    yavs> force enable '1'
    tb.enable = '1'
    yavs> run 20 ns
    step at 42 ns, delta 0
    yavs> examine q
    '1'
    yavs> drivers enable
    tb.enable : STD_LOGIC = '1', declared at tb.vhd:29
      no driver: it keeps its initial value unless forced or deposited
      resolved by resolved
      force: '1'
    yavs> force enable 1
    tb.enable = '1'
    yavs> force cnt 0101
    tb.cnt = "0101"
    yavs> force enable
    error: `force` needs a value
    yavs> force enable -out '1'
    error: `-out '1'` is not a value of `tb.enable`, type STD_LOGIC: expected one of 'U', 'X', '0', '1', 'Z', 'W', 'L', 'H', '-': a flag goes last, after the value
    yavs> force nosuch '1'
    error: no signal `nosuch` here
    yavs> force cnt "10"
    error: `"10"` is not a value of `tb.cnt`, type UNSIGNED: expected 4 elements, such as 0101 or "0101", x".." or an aggregate
    yavs> force q 'Z' -out
    tb.q = 'Z'
    yavs> run 1 ns
    step at 43 ns, delta 0
    yavs> drivers q
    tb.q : STD_LOGIC = 'Z', declared at tb.vhd:30
      driver           value  assigned at
      port tb.u_reg.q  '1'    tb.vhd:7
      resolved by resolved
      force out: 'Z'

.. _console-deposit:

deposit
^^^^^^^

.. console: deposit

.. code-block:: text

    deposit NAME VALUE

Give a signal a value once, in the next cycle.

.. console-end

``deposit`` gives the signal the value once, in the next cycle: its drivers may change it at their
next assignment. ``-out`` has no meaning for a deposit and is an error.

.. example:

.. code-block:: text

    yavs> run 12 ns
    step at 12 ns, delta 0
    yavs> deposit cnt "0101"
    tb.cnt = "0101"
    yavs> deposit cnt(1 downto 0) "11"
    tb.cnt(1 downto 0) = "11"
    yavs> deposit cnt(9) '0'
    error: index 9 is out of range
    yavs> deposit cnt "0101" -out
    error: `deposit` has no `-out`: a deposit gives a value once, to the signal
    yavs> step delta
    step at 12 ns, delta 1
    yavs> examine cnt
    "0111"
    yavs> run 10 ns
    the simulation ended at 20 ns, delta 0
    yavs> examine cnt
    "1000"

.. _console-release:

release
^^^^^^^

.. console: release

.. code-block:: text

    release NAME [-out]

End a force, or with `-out` only one on the driving value.

.. console-end

``release`` ends the force on the signal, and the signal takes the value its drivers give it, or
keeps its value if it has none. With ``-out`` it ends the force on the driving value only; without,
either force. A release of a signal that is not forced is not an error. The reply
is the path.

Values of every kind, on ``types.vhd``:

.. example: top=types_tb

.. code-block:: text

    yavs> force state done
    types_tb.state = done
    yavs> force count 16#2A#
    types_tb.count = 42
    yavs> force count 300
    error: `300` is not a value of `types_tb.count`, type INTEGER: expected an integer from 0 to 255
    yavs> force ratio 1.5e-3
    types_tb.ratio = 0.0015
    yavs> force delay 5 us
    types_tb.delay = 5 us
    yavs> force delay 5
    error: `5` is not a value of `types_tb.delay`, type TIME: expected a number and a unit, such as 10 fs
    yavs> force pair (x => 3, ok => true, tag => 'Z')
    types_tb.pair = (x => 3, ok => TRUE, tag => 'Z')
    yavs> force pair (3, true, 'Z')
    types_tb.pair = (x => 3, ok => TRUE, tag => 'Z')
    yavs> force pair.ok false
    types_tb.pair.ok = FALSE
    yavs> force words (1 => x"FF", others => x"00")
    types_tb.words = ("00000000", "11111111", "00000000", "00000000")
    yavs> force words(2) x"AA"
    types_tb.words(2) = "10101010"
    yavs> force bus8 b"1010_1010"
    types_tb.bus8 = "10101010"
    yavs> force bus8 "0101"
    error: `"0101"` is not a value of `types_tb.bus8`, type STD_LOGIC_VECTOR: expected 8 elements, such as 01010101 or "01010101", x".." or an aggregate
    yavs> force bus8 (7 downto 4 => '1', others => '0')
    types_tb.bus8 = "11110000"
    yavs> force bus8(9) '1'
    error: index 9 is out of range
    yavs> force bus8(7 downto 4) "1111"
    types_tb.bus8(7 downto 4) = "1111"
    yavs> release bus8
    types_tb.bus8
    yavs> step delta
    step at 0 fs, delta 1
    yavs> examine pair
    (x => 3, ok => FALSE, tag => 'Z')
    yavs> examine words
    ("00000000", "11111111", "10101010", "00000000")
    yavs> examine delay
    5 us

Waves
~~~~~

The ``wave`` commands drive the viewer started by ``--surfer`` (:doc:`waveforms`); without it
every one of them is an error that says to run with ``--surfer`` (whatever follows ``wave``):

.. example:

.. code-block:: text

    yavs> wave add cnt
    error: `wave` needs a viewer: run with `--surfer`
    yavs> wave cursor 12 ns
    error: `wave` needs a viewer: run with `--surfer`
    yavs> wave
    error: `wave` needs a viewer: run with `--surfer`

The viewer shows the signals with their history since time 0, read from the wave file, so it
follows ``--wave-include``, ``--wave-exclude`` and ``--wave-max-array``. A wrong subcommand is an
error with the closest one.

.. _console-wave:

wave
^^^^

.. console: wave

.. code-block:: text

    wave add NAME|PATTERN...
    wave cursor [TIME]
    wave save [FILE]
    wave load [FILE]

With `--surfer`, show signals or scopes in the viewer; move its cursor (default: now); save or load the signal list. Needs a viewer: run with ``--surfer``.

.. console-end

.. _console-wave-add:

wave add
""""""""

.. console: wave add

.. code-block:: text

    wave add NAME|PATTERN...

Needs a viewer: run with ``--surfer``.

.. console-end

Adds to the view, in this order of the line, every ``NAME`` or ``PATTERN``, separated by spaces
(spaces inside parentheses belong to the name). A name is a signal, a part of one, a variable of
the process stopped in, a process variable by its path, or a scope, which adds the signals of
that scope and not those below. A word with ``*`` or ``?`` is a pattern as in ``find signals``;
every signal it matches is added, and if it matches none the reply is the error of ``find``. The
reply is the number of items the viewer added. Errors: no argument; a name the wave file does
not hold (it says so, and names the wave options that leave signals out); a local of a
subprogram, which exists only during a call.

.. _console-wave-cursor:

wave cursor
"""""""""""

.. console: wave cursor

.. code-block:: text

    wave cursor [TIME]

Needs a viewer: run with ``--surfer``.

.. console-end

Moves the viewer's cursor to ``TIME`` (`Time`_, absolute), or to the time of the stop without
one. It prints nothing.

.. _console-wave-save:

wave save
"""""""""

.. console: wave save

.. code-block:: text

    wave save [FILE]

Needs a viewer: run with ``--surfer``.

.. console-end

Writes the signals the view shows, one dotted name per line, to ``FILE``; without it to
``yavs-wave-layout/NAME.txt`` in the work library of the run (``NAME`` the testbench). The reply is
the number saved and the file. A run without a work library needs a file name.

.. _console-wave-load:

wave load
"""""""""

.. console: wave load

.. code-block:: text

    wave load [FILE]

Needs a viewer: run with ``--surfer``.

.. console-end

Adds the signals listed in ``FILE`` (or the testbench's file of ``wave save``), one by one. The
reply says how many were added and from which file, and names the entries this run does not
record (``not in this design: NAME``). A file that cannot be read is an error that points to
``wave save``.

.. example: --wave=tb.fst --surfer

.. code-block:: text

    yavs> run 20 ns
    note: the waveform is written to `tb.fst`
    step at 20 ns, delta 0
    yavs> wave add cnt
    1 added
    yavs> wave add *clk* tb.counter.total
    3 added
    yavs> wave add nosuch
    error: `tb.nosuch` is not in the waveform file: only signals, process variables and shared variables are, without what `--wave-include`, `--wave-exclude` or `--wave-max-array` leave out
    yavs> wave cursor 12 ns
    yavs> wave cursor xx
    error: `xx` is not a time: "xx" is not a unit of the type
    yavs> wave save
    9 signals saved to `lib/yavs-wave-layout/tb.txt`
    yavs> wave load nofile.txt
    error: cannot read the wave layout `nofile.txt`: No such file or directory (os error 2); `wave save` writes one
    yavs> wave foo
    error: `wave foo`: expected `add` or `cursor` or `save` or `load`

Find
~~~~

.. _console-find:

find
^^^^

.. console: find

.. code-block:: text

    find signals PATTERN
    find variables PATTERN

Search the hierarchy; `*` within a level, `**` across, a pattern without a dot matches the last name.

.. console-end

``PATTERN`` is a glob: ``*`` matches any characters within one level of the hierarchy, ``**``
across levels, ``?`` one character. A pattern without a dot is matched against the last name of
every path (``find signals *clk*``); one with a dot against the whole path (``find signals
u_reg.*``), and also against the end of it (``u_reg.q`` finds ``tb.u_reg.q``). Case does not
matter. The singular ``signal`` and ``variable`` work too. No match is an error that suggests the
closest names.

.. _console-find-signals:

find signals
""""""""""""

.. console: find signals

.. code-block:: text

    find signals PATTERN

.. console-end

Prints a table of the signals and ports that match: path, kind, type and value. What
it lists, ``examine``, ``drivers``, ``describe`` and ``wave add`` take.

.. example:

.. code-block:: text

    yavs> find signals *clk*
      signal        kind           type       value
      tb.clk        signal         STD_LOGIC  '0'
      tb.u_reg.clk  port, mode in  STD_LOGIC  '0'
    yavs> find signals u_reg.*
      signal        kind            type       value
      tb.u_reg.clk  port, mode in   STD_LOGIC  '0'
      tb.u_reg.d    port, mode in   STD_LOGIC  'U'
      tb.u_reg.q    port, mode out  STD_LOGIC  'U'
    yavs> find signals **.q
      signal      kind            type       value
      tb.q        signal          STD_LOGIC  'U'
      tb.u_reg.q  port, mode out  STD_LOGIC  'U'
    yavs> find signals zz*
    error: no signal matches `zz*`
    yavs> find
    error: `find` needs a subcommand: `signals` or `variables`

.. _console-find-variables:

find variables
""""""""""""""

.. console: find variables

.. code-block:: text

    find variables PATTERN

.. console-end

Prints a table of the variables declared in processes and the shared variables of the design: path
and kind. A variable of a subprogram is not listed.

.. example:

.. code-block:: text

    yavs> find variables *
      variable          kind
      tb.counter.total  variable of tb.counter
    yavs> find variables tot*
      variable          kind
      tb.counter.total  variable of tb.counter
    yavs> find vars x
    error: `find vars`: expected `signals` or `variables`

Diagnose
~~~~~~~~

.. _console-drivers:

drivers
^^^^^^^

.. console: drivers

.. code-block:: text

    drivers NAME

What drives a signal: each driver's value and source line, the resolved value, any force.

.. console-end

Prints the signal's type and value, then one row per driver: the process (or port) that drives
it, its value and the line it assigns at. Then the resolution function, and a force if there
is one. A signal with no driver says so.

.. example: --wave=tb.fst

.. code-block:: text

    yavs> run 30 ns
    ** Error: 25000000fs+1: q is not high
        Process tb.check
    step at 30 ns, delta 0
    yavs> drivers q
    tb.q : STD_LOGIC = 'U', declared at tb.vhd:30
      driver           value  assigned at
      port tb.u_reg.q  'U'    tb.vhd:7
      resolved by resolved
    yavs> drivers level
    tb.level : STD_LOGIC_VECTOR = "X1", declared at tb.vhd:32
      driver                               value  assigned at
      process tb.level <= ... (tb.vhd:47)  "01"   tb.vhd:47
      process tb.level <= ... (tb.vhd:48)  "11"   tb.vhd:48
      resolved by resolved (per element)
    yavs> drivers enable
    tb.enable : STD_LOGIC = 'U', declared at tb.vhd:29
      no driver: it keeps its initial value unless forced or deposited
      resolved by resolved
    yavs> drivers tb.u_reg.clk
    tb.u_reg.clk : STD_LOGIC = '0', declared at tb.vhd:7
      no driver of its own: this port takes its value from tb.clk
      resolved by resolved
    yavs> drivers
    error: `drivers` needs a signal name
    yavs> drivers cnt(0)
    error: no signal `cnt(0)` here
    yavs> drivers nosuch
    error: no signal `nosuch` here

.. _console-waits:

waits
^^^^^

.. console: waits

.. code-block:: text

    waits

Where each process stands and what it waits for.

.. console-end

Prints one row per process: its state (``waiting``, ``ended`` after ``wait;``, ``running``,
``not started``, ``stopped here``), the signals and time it waits for, and the line of the
``wait``. It works at any stop.

.. example:

.. code-block:: text

    yavs> waits
      process                      state        waits for  at
      tb.clk <= ... (tb.vhd:34)    not started
      tb.u_reg.flop                not started
      tb.counter                   not started
      tb.level <= ... (tb.vhd:47)  not started
      tb.level <= ... (tb.vhd:48)  not started
      tb.check                     not started
    yavs> run 12 ns
    step at 12 ns, delta 0
    yavs> waits
      process                      state    waits for         at
      tb.clk <= ... (tb.vhd:34)    waiting  tb.clk, tb.cnt    tb.vhd:34
      tb.u_reg.flop                waiting  tb.u_reg.clk      tb.vhd:12
      tb.counter                   waiting  tb.clk            tb.vhd:38
      tb.level <= ... (tb.vhd:47)  ended    nothing: `wait;`  tb.vhd:47
      tb.level <= ... (tb.vhd:48)  waiting  tb.cnt            tb.vhd:48
      tb.check                     waiting  tb.cnt            tb.vhd:52

.. _console-trace:

trace
^^^^^

.. console: trace

.. code-block:: text

    trace x NAME

Follow an unknown value back through drivers and the signals they read.

.. console-end

.. _console-trace-x:

trace x
"""""""

.. console: trace x

.. code-block:: text

    trace x NAME

.. console-end

Follows an unknown or uninitialised value of ``NAME`` back along the drivers of the signal, and from
a process to the signals its statements read, and prints the chain as a tree with the value of
each and since when it has been unknown. Under ``Where it starts`` it names the places the
chain ends: nothing drives a signal, two drivers hold opposite values, a force, or a process
with no unknown input. With a wave file (``--wave``) it also says since when; without one, when
it last changed. It cannot see variables, values from files, subprogram parameters, or which
signal read made the value. A signal with no unknown value says so.

.. example: --wave=tb.fst

.. code-block:: text

    yavs> run 30 ns
    ** Error: 25000000fs+1: q is not high
        Process tb.check
    step at 30 ns, delta 0
    yavs> trace x q
    tb.q = 'U', since 0 fs, declared at tb.vhd:30
      tb.u_reg.q = 'U', since 0 fs, declared at tb.vhd:7
        driven 'U' by process tb.u_reg.flop, at tb.vhd:15
          tb.u_reg.d = 'U', since 0 fs, declared at tb.vhd:7
            tb.enable = 'U', since 0 fs, declared at tb.vhd:29
    Where it starts:
      tb.enable: nothing drives it, it keeps its initial value (tb.vhd:29)
    It follows drivers and the signals their processes name. It cannot see variables,
    values from files, subprogram parameters, or which signal read made this value.
    yavs> trace x cnt
    tb.cnt = "0011": all known, nothing to trace
    yavs> trace
    error: `trace` needs a subcommand: `x`
    yavs> trace y q
    error: `trace y`: expected `x`

.. _console-activity:

activity
^^^^^^^^

.. console: activity

.. code-block:: text

    activity

The processes that ran most often and the signals that changed most often, so far.

.. console-end

Prints two tables: the processes that ran most often (runs, share, name, place) and the
signals with the most delta cycles with an event. Activations the kernel avoids do not run, so
they do not count. At most the busiest entries are shown, and the totals say how many there
were. The same tables at the end of a run: ``--activity``.

.. _console-asserts:

asserts
^^^^^^^

.. console: asserts

.. code-block:: text

    asserts

Assertion failures and reports so far, by severity and source line.

.. console-end

Prints the counts of failed assertions and reports by severity, then a row per place:
severity, count, source line and the first message. Counts are per innermost statement. The same
at the end of a run: ``--assert-summary``.

.. example:

.. code-block:: text

    yavs> asserts
    Assertion summary: 0 failures, 0 errors, 0 warnings, 0 notes
      no assertion failed and no report was made
    yavs> run 30 ns
    ** Error: 25000000fs+1: q is not high
        Process tb.check
    step at 30 ns, delta 0
    yavs> asserts
    Assertion summary: 0 failures, 1 error, 0 warnings, 0 notes
      severity  count  place      first message
      error         1  tb.vhd:53  q is not high
    yavs> activity
    Process activations: 33 runs in all, 6 shown
      runs      %  name                         place
        10   30.3  tb.clk <= ... (tb.vhd:34)    tb.vhd:34
         7   21.2  tb.counter                   tb.vhd:41
         7   21.2  tb.u_reg.flop                tb.vhd:14
         4   12.1  tb.check                     tb.vhd:52
         4   12.1  tb.level <= ... (tb.vhd:48)  tb.vhd:48
         1    3.0  tb.level <= ... (tb.vhd:47)  tb.vhd:47
    Signal activity: 17 events in all, 4 shown
      events      %  name          place
           6   35.3  tb.clk        tb.vhd:28
           6   35.3  tb.u_reg.clk  tb.vhd:7
           3   17.6  tb.cnt        tb.vhd:31
           2   11.8  tb.level      tb.vhd:32

Session and scripts
~~~~~~~~~~~~~~~~~~~

.. _console-restart:

restart
^^^^^^^

.. console: restart

.. code-block:: text

    restart

Back to time 0 with the same design and breakpoints. Also written ``reset``.

.. console-end

Goes back to time 0 with the same elaborated design, and stops before any process has run.
Breakpoints stay; nothing is compiled again; a wave file is written again from the start. With
``--vhpi`` or ``--load`` it is refused with the reason: run again. ``reset`` is the same.

.. example:

.. code-block:: text

    yavs> break tb.vhd:43
    breakpoint 1: /work/tb.vhd:43
    yavs> run
    breakpoint at 5 ns, delta 0, /work/tb.vhd:43
    yavs> run
    breakpoint at 15 ns, delta 0, /work/tb.vhd:43
    yavs> restart
    yavs> where
    time 0 fs, delta 0
    between cycles
    yavs> break list
    1 /work/tb.vhd:43
    yavs> run
    breakpoint at 5 ns, delta 0, /work/tb.vhd:43

.. _console-source:

source
^^^^^^

.. console: source

.. code-block:: text

    source FILE

Run the commands of a file; the first failure ends it.

.. console-end

Reads a file of console lines (relative to where yavs runs) and runs them as if typed, one after
the other. The first command that fails ends the file, with its error, and with it the line or
the script that called ``source``: in ``source fail.do; examine q`` the last command does not run.
Comments (`Lines and commands`_) and empty lines are fine, and a file may ``source`` another. A
file that cannot be read is an error naming it.

.. example-shell:

.. code-block:: text

    $ cat setup.do
    # Stop in the counter and look at it.
    break tb.vhd:43
    run
    examine total
    examine cnt -- the signal, before the statement ran
    $ yavs run --work=work:lib tb --do "source setup.do"; echo "status $?"
    breakpoint 1: /work/tb.vhd:43
    breakpoint at 5 ns, delta 0, /work/tb.vhd:43
    1
    "0000"
    status 0
    $ cat fail.do
    examine cnt
    examine nosuch
    examine clk
    $ yavs run --work=work:lib tb --do "source fail.do; examine q"; echo "status $?"
    "0000"
    error: no variable or signal `nosuch` here
    status 1
    $ yavs run --work=work:lib tb --interactive --do "source fail.do; examine q" < /dev/null; echo "status $?"
    "0000"
    error: no variable or signal `nosuch` here
    status 0
    $ yavs run --work=work:lib tb --do "source nofile.do"; echo "status $?"
    error: cannot read `nofile.do`: No such file or directory (os error 2)
    status 1

.. _console-help:

help
^^^^

.. console: help

.. code-block:: text

    help
    help COMMAND

This list, or one command.

.. console-end

``help`` alone prints every command with its synopsis and meaning, grouped as on this page, and the
commands of the test runner if there is one: the meanings in one column, long lines wrapped to the width of the terminal,
and command names in bold on a colour terminal and in an editor's Debug Console (never in piped output).
``help COMMAND`` prints the synopsis and meaning of one command (``print`` and ``reset`` give those
of ``examine`` and ``restart``), and the help text of a test runner's command. A word that is no
command is an error with the closest command.

.. example:

.. code-block:: text

    yavs> help break
    break [FILE:]LINE [if CONDITION]
    break SIGNAL [= VALUE] [if CONDITION]
    break assert [note|warning|error|failure]
    break list
    break delete N
    break delete assert
    break delete all
        Stop at a line, at a change of a signal, or at a report of this severity and worse; list or delete them.
    yavs> help print
    examine NAME
    examine NAME @ TIME
        The value of a local or a signal, or a field, element or slice of one; with a time, from the wave file.
        Also written `print`.
        `examine NAME @ TIME` needs a wave file: run with `--wave=FILE.fst`.
    yavs> help wave
    wave add NAME|PATTERN...
    wave cursor [TIME]
    wave save [FILE]
    wave load [FILE]
        With `--surfer`, show signals or scopes in the viewer; move its cursor (default: now); save or load the signal list.
        `wave add NAME|PATTERN...` needs a viewer: run with `--surfer`.
        `wave cursor [TIME]` needs a viewer: run with `--surfer`.
        `wave save [FILE]` needs a viewer: run with `--surfer`.
        `wave load [FILE]` needs a viewer: run with `--surfer`.
    yavs> help brek
    error: no command `brek`: a command with a similar name exists: `break`
    yavs> help
    Run and step:
      run | run TIME | run until CONDITION
                                    to the end or a breakpoint, for TIME, or until a
                                    change of a signal in the condition makes it
                                    true
      step | step delta | step timestep
                                    to the next statement, entering calls; or to the
                                    end of the delta or time step
      next                          to the next statement, over calls
      stop                          pause a running simulation (Ctrl-C on a
                                    terminal)
    Breakpoints:
      break [FILE:]LINE [if CONDITION] | break SIGNAL [= VALUE] [if CONDITION] |
        break assert [note|warning|error|failure] | break list | break delete N |
        break delete assert | break delete all
                                    stop at a line, at a change of a signal, or at a
                                    report of this severity and worse; list or
                                    delete them
    Inspect:
      examine NAME | examine NAME @ TIME
                                    the value of a local or a signal, or a field,
                                    element or slice of one; with a time, from the
                                    wave file (also `print`)
      describe NAME                 type, bounds, resolution function, mode and
                                    declaration of a signal or variable
      where                         time, delta, process and source line
    Change values:
      force NAME VALUE [-out]       hold a signal at a value, from the next cycle;
                                    with `-out` the driving value, not what readers
                                    see
      deposit NAME VALUE            give a signal a value once, in the next cycle
      release NAME [-out]           end a force, or with `-out` only one on the
                                    driving value
    Waves:
      wave add NAME|PATTERN... | wave cursor [TIME] | wave save [FILE] | wave load
        [FILE]
                                    with `--surfer`, show signals or scopes in the
                                    viewer; move its cursor (default: now); save or
                                    load the signal list
    Find:
      find signals PATTERN | find variables PATTERN
                                    search the hierarchy; `*` within a level, `**`
                                    across, a pattern without a dot matches the last
                                    name
    Diagnose:
      drivers NAME                  what drives a signal: each driver's value and
                                    source line, the resolved value, any force
      waits                         where each process stands and what it waits for
      trace x NAME                  follow an unknown value back through drivers and
                                    the signals they read
      activity                      the processes that ran most often and the
                                    signals that changed most often, so far
      asserts                       assertion failures and reports so far, by
                                    severity and source line
    Session and scripts:
      restart                       back to time 0 with the same design and
                                    breakpoints (also `reset`)
      source FILE                   run the commands of a file; the first failure
                                    ends it
      help | help COMMAND           this list, or one command
      quit [STATUS]                 leave; the exit status is STATUS, else that of
                                    the ended run, else 0 (also `exit`)

.. _console-quit:

quit
^^^^

.. console: quit

.. code-block:: text

    quit [STATUS]

Leave; the exit status is STATUS, else that of the ended run, else 0. Also written ``exit``.

.. console-end

Ends the session. ``STATUS`` is the exit status of ``yavs``, 0 to 255; without it the
status is that of the simulation if it has ended (1 after a reported error, as in a plain run),
else 0. A value outside 0..255 is an error and the session goes on. ``exit`` is the same command. Ctrl-D at an empty prompt and the end of
piped input are ``quit``. With ``--do`` and no ``--interactive`` the script ends with an implicit
``quit``.

.. example:

.. code-block:: text

    yavs> quit 300
    error: `quit 300`: expected a status 0..255
    yavs> quit now
    error: `quit now`: expected a status 0..255
    yavs> quit 2

Test runner commands
~~~~~~~~~~~~~~~~~~~~

A test runner can add commands of its own to the console (the protocol is in the `command line
contract <https://github.com/ru551n/yavs/blob/main/docs/cli-contract.md>`__). They exist only
through the test runner's interface: with the Python package ``vunit-yavs``, ``python run.py
lib.tb.test --yavs-interactive`` starts the test at the prompt with these commands added (:doc:`vunit-yavs`).
Typed at a plain ``yavs run --interactive``, a ``vunit_*`` command is an error that says so:

.. example:

.. code-block:: text

    yavs> vunit_help
    error: `vunit_help` is only available through the test runner interface: start the test with `python run.py lib.tb.test --yavs-interactive` (vunit-yavs), not with `yavs run --interactive`
    yavs> vunit_load --stop-time=30ns
    error: `vunit_load` is only available through the test runner interface: start the test with `python run.py lib.tb.test --yavs-interactive` (vunit-yavs), not with `yavs run --interactive`

.. include:: console-runner-commands.inc

See also
--------

* :doc:`interactive`: the prompt, worked examples and options.
* :doc:`debugger-tutorial`: a failing test debugged step by step.
* :doc:`debugging`: the same commands in the Debug Console of an editor.
* :doc:`waveforms`: ``--surfer`` and the ``wave`` commands.
* :doc:`vunit-yavs`: the commands a test runner adds.

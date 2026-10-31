..
   Generated from the rule list: YAVS_BLESS=1 cargo test -p yavs --bin yavs cli_reference

Lint rules
==========

Every rule of :doc:`yavs lint <lint>`. ``yavs lint --explain RULE`` prints the same.

lint_001
--------

advisory, off by default. A signal read by a combinational process is missing from its sensitivity list.

The process does not run when that signal changes, so what it computes goes stale and simulation differs from the logic the text describes. A process that tests a clock edge or suspends on ``wait`` is not judged.

lint_002
--------

advisory, off by default. A signal in a sensitivity list is not read by the process.

The process runs for nothing when the signal changes, or something it was meant to read is missing. A process that calls a subprogram is not judged, as the subprogram may read the signal by name.

lint_004
--------

advisory, off by default. A declaration is never used.

A signal, constant or variable of an architecture that nothing names. A declaration of a package can be used anywhere, and types and subprograms are not judged.

lint_005
--------

advisory, off by default. ``library work`` is implicit and does not need declaring.

Every design unit has it. A clause that repeats it says nothing.

lint_006
--------

advisory, off by default. A context clause is not attached to any design unit.

A context clause belongs to the unit after it. Analysis already rejects one with no unit after it, so a lint run shows that error and this rule never adds a finding of its own; it stays so that a configuration can name it.

lint_600
--------

experimental, off by default. A combinational process does not assign a signal on every path, inferring a latch.

The signal keeps its value on the path that does not assign it, which a synthesis tool builds as a latch. A process is taken as combinational when it tests no edge and has no ``wait`` or procedure call. A signal assigned first has a default and is not reported. A signal assigned in part or inside a loop is not judged.

lint_601
--------

advisory, off by default. A signal is assigned by more than one concurrent statement.

Legal on a resolved type, where resolution decides the value rather than the design. Deliberate tri-state buses are reported too: waive those. Parts are compared by value, so ``q(3)`` and ``q(4)`` are two signals; parts a generic decides do not count, and neither do statements under different generate alternatives, as a generic decides which exist. An output of an instance is a driver, an ``inout`` too.

lint_602
--------

policy, off by default. A signal assigned by a clocked process does not have a register suffix (set ``suffixes``).

A naming convention. ``suffixes`` lists what is accepted, ``*`` and ``?`` allowed; the default is ``_q``, ``_r``, ``_reg``.

lint_603
--------

policy, off by default. A signal assigned by a clocked process does not have a register prefix (set ``prefixes``).

A naming convention. ``prefixes`` lists what is accepted, ``*`` and ``?`` allowed; the default is ``r_``.

lint_700
--------

experimental, off by default. A signal registered on one clock is used in logic on another, without a synchroniser.

The receiving flop can sample the signal while it changes and go metastable. The clock of a register is the edge its process tests; an architecture needs two clocks for this to be asked. A capture into a flop and nothing else counts as the first stage of a synchroniser, so a single stage is accepted, and a signal on two clocks is skipped.

lint_701
--------

experimental, off by default. A register reset by one signal is used in logic reset by another.

The resets can release at different moments, so the destination captures from a register that is mid-release. Only an asynchronous reset counts: the branch a clock edge is the ``elsif`` of. A capture into a flop and nothing else is accepted; a register cleared by two resets is skipped.

lint_702
--------

advisory, off by default. A register a clocked process assigns but its reset branch does not.

The reset branch says what the design starts from, and this register starts from what it holds. Often an oversight, sometimes a saving on a wide data path. Only an asynchronous reset is judged: the branch a clock edge is the ``elsif`` of.

lint_703
--------

advisory, off by default. A clock used on both its rising and its falling edge.

It halves the time between the two sets of registers. Sometimes that is the design: double data rate logic, or an interface that samples on one edge and shifts on the other. An edge in a ``wait`` is a process suspending, and is not counted.

lint_710
--------

advisory, off by default. A state of an enumerated state machine is never entered.

No assignment gives this value to a signal of an enumeration the architecture declares and a ``case`` selects on, and no initial value does. The machine has to be readable: a value that does not fold, or a signal driven through a port or a parameter, leaves the type alone.

lint_711
--------

advisory, off by default. A state of an enumerated state machine has no exit.

The ``case`` alternative of the state never assigns another state, so the machine cannot leave it unless it is a deliberate terminal state: waive that. A ``case`` that assigns no state at all is a multiplexer and is not checked, and neither is one after a default assignment, as the default is then the exit.

lint_712
--------

advisory, off by default. A ``when others`` alternative that no value can reach.

The other alternatives already name every value of the selector. Add a value to the enumeration and the new value falls into ``others``; without it, the same edit is an error naming the missing choice. The selector must be an object of an enumeration type with every choice folded.

lint_713
--------

policy, off by default. A ``when others`` alternative on an enumeration, instead of naming every value.

While ``others`` is there, a new value of the enumeration quietly takes that branch. A project that wants every value spelled out enables this. It takes the ``others`` alternatives that do stand in for values, and lint_712 those that do not.

lint_720
--------

advisory, off by default. A signal depends on itself through combinational logic, with no register in the loop.

Hardware oscillates or settles somewhere nobody chose. A clocked process gives no dependency. Left out so that a cycle is a real one: assignments to and reads of part of an object, anything through an instance, attributes, and assignments with an ``after``.

lint_730
--------

advisory, off by default. A signal is read but nothing drives it: no assignment, and no instance output.

It holds its initial value for ever: usually a wire someone forgot to connect. A signal with an initial value is taken as tied on purpose; a subprogram that names the signal may drive it, so that counts as driven.

lint_740
--------

definite error, on by default. A vector is assigned to one of a different width.

Both sides are of one array type, so analysis has nothing to say; the lengths are compared when the assignment runs, and it fails. Reported when the lengths of the target and of the value are both known: an object or a slice with a range that folds, or a literal. A width that depends on a generic, a function result or an operator is not judged.

lint_741
--------

definite error, on by default. A vector is compared with one of a different width, which is never equal.

The predefined ``=`` is true only for equal lengths with matching elements, so comparing a four-bit vector with a three-bit literal is legal and always false (``/=`` always true). Reported for the predefined operators only: an overloaded ``=`` such as the one of an arithmetic package may resize its operands. Both lengths must be known, as for lint_740.

lint_750
--------

advisory, off by default. A component declaration does not match the entity it stands for.

A component is a hand-written copy of an entity's interface, and copies drift. Port names, modes and order are compared, not types. Only an entity of that name in the same library, analyzed in this run, is compared, so a component standing for a primitive is left alone.

lint_751
--------

definite error, on by default. A binding indication or an instance names an architecture that is not declared.

``use entity work.e(a)`` in a configuration and ``i : entity work.e(a)`` in an architecture name architecture ``a`` of ``e``, and elaboration fails when the library has no such architecture. Analysis accepts them, as the architecture may come later. The library's own list is the evidence, so analyze the whole library first. A block configuration ``for a`` with no such architecture is an analysis error, E1002.

lint_760
--------

advisory, off by default. A subprogram whose body calls itself, which no synthesis tool accepts.

Recursion is legal and simulates, and code only ever simulated may be right to use it. For code that has to synthesise, the call has no bottom to inline from. The call is resolved, so another subprogram of the same name does not count. Direct recursion only.

lint_770
--------

definite error, on by default. A process with no sensitivity list and no wait statement, which can never suspend.

A process repeats for ever; it lets time advance only by suspending, at a ``wait`` or at the end of its body when it has a sensitivity list. With neither, simulation time never advances. A process that calls a procedure is left alone, because the procedure may hold the ``wait``. A ``wait`` that looks unreachable still counts.

lint_771
--------

definite error, on by default. A function that can reach the end of its body without returning a value.

Reaching the end of a function is an error, reported only when a run takes that path with those arguments. Anything unclear counts as an ending, so correct code is not accused: a loop is assumed to return, and so is an assertion or report that always fails, and a procedure call that ends the body, which may stop the run. A ``case`` covers its subtype, so it needs no ``others``.

lint_772
--------

definite error, on by default. A statement that nothing can reach.

The statement follows a ``return``, or an ``exit`` or ``next`` with no condition, or an ``if`` or ``case`` all of whose paths do one of those. An assertion is no transfer of control, even at severity failure. A ``translate_off`` or ``synthesis_off`` pragma between the two statements makes this no finding: code unreachable in simulation and reachable in synthesis is how a design asks which tool reads it.

lint_780
--------

definite error, on by default. An index that is outside the declared range of the array it selects from.

Evaluating the index raises an error. Reported when the index and the range of the array both fold; ``yavs analyze`` warns about some of the same (W1404), and this rule makes it a finding of the lint run.

lint_781
--------

definite error, on by default. A division, ``mod`` or ``rem`` whose divisor is zero.

The right operand of the predefined ``/``, ``mod`` and ``rem`` may not be zero: evaluating it raises an error. Reported when the divisor folds to zero. An overloaded operator defines its own behaviour and is not judged.

lint_782
--------

definite error, on by default. An assignment of a value outside the declared range of its target.

The value has to belong to the target's subtype, so the assignment raises an error when it runs. Analysis already rejects this when the value and the range both fold, as E1403, so a lint run shows that error and this rule never adds a finding of its own; it stays so that a configuration can name it.

lint_790
--------

advisory, off by default. A signal declared in a wider region than the one that uses it.

Where a signal is declared says who it belongs to. Declared in the architecture and used only inside one block or generate body, it reads as shared state when it is not; in a ``for ... generate`` it is one signal for every iteration, where one per iteration was probably meant. The guard and maps of a block and the header of a generate are read in the region around them. An unused signal, and one with an attribute specification, are not reported.

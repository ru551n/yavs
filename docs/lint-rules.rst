..
   Generated from the rule list: YAVS_BLESS=1 cargo test -p yavs --bin yavs cli_reference

Lint rules
==========

Every rule of :doc:`yavs lint <lint>`. ``yavs lint --explain RULE`` prints the same.

lint_740
--------

definite error, on by default. A vector is assigned to one of a different width.

Both sides are of one array type, so analysis has nothing to say; the lengths are compared when the assignment runs, and it fails. Reported when the lengths of the target and of the value are both known: an object or a slice with a range that folds, or a literal. A width that depends on a generic, a function result or an operator is not judged.

lint_741
--------

definite error, on by default. A vector is compared with one of a different width, which is never equal.

The predefined ``=`` is true only for equal lengths with matching elements, so comparing a four-bit vector with a three-bit literal is legal and always false (``/=`` always true). Reported for the predefined operators only: an overloaded ``=`` such as the one of an arithmetic package may resize its operands. Both lengths must be known, as for lint_740.

lint_751
--------

definite error, on by default. A configuration names an architecture that is not declared.

A binding indication ``use entity work.e(a)`` names architecture ``a`` of ``e``, and elaboration would fail when the library has no such architecture. Analysis already rejects it, as E1002, so a lint run shows that error and this rule never adds a finding of its own; it stays so that a configuration can name it.

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

Where simulators differ
=======================

The VHDL standard leaves some points open, and simulators do not always agree on them. Where the
standard is clear, yavs follows it and reports a violation early; where it is open, yavs is
permissive. These are the differences you are most likely to run into when you move a design to
yavs, with what to do about each.

.. list-table::
   :header-rows: 1
   :widths: 30 40 30

   * - Situation
     - What yavs does
     - What you can do
   * - A testbench depends on which process runs first in a cycle
     - Runs processes in one fixed order, the same at any ``--threads``.
     - ``--shuffle[=SEED]`` runs another order to find the dependency; pass values through
       signals instead.
   * - Verification code with a shared variable that is not protected, a pure function that calls
       an impure one, or an ``others`` aggregate assigned to an unconstrained object
     - Errors, as the standard says.
     - ``--relaxed`` on ``analyze`` and ``run`` turns them, and the other cases below that name it,
       into warnings.
   * - A PSL ``sequence`` or ``property`` declaration after ``begin``
     - Error ``E1241``: VHDL-2008 makes it a declaration, written before ``begin``.
     - Move it before ``begin``, or pass ``--relaxed`` (warning ``W1241``).
   * - PSL in ``-- psl`` comments
     - Read only when asked.
     - ``--psl`` on ``analyze`` and ``run``.
   * - A TIME literal with a unit below ``--resolution`` (``1 ps`` with ``--resolution=ns``)
     - Error before time 0, also in code that never runs.
     - Use a finer ``--resolution`` (default ``fs``) or write times in coarser units.
   * - Files of different standards in one design
     - Allowed; the design uses the packages of the newest standard.
     - Keep VHDL-1993 code that overloads on both ``std_logic_vector`` and ``std_ulogic_vector``
       in a design of 1993 files only.
   * - A VHDL-1993 file that uses ``=`` on ``std_logic_vector`` with both ``std_logic_1164`` and
       ``std_logic_unsigned`` visible (or any operation declared implicitly by one package and
       explicitly by another)
     - Error ``E1302`` under ``--std=93``: VHDL-1993 hides neither declaration.
     - ``--relaxed`` uses the explicit one, as VHDL-2008 does (warning ``W1302``); or analyze the
       file with ``--std=2008``.
   * - VHDL-1993 code that relies on rules VHDL-2008 relaxed
     - Error ``E1007`` under ``--std=93``, naming the 1993 rule.
     - Analyze the file with ``--std=2008``.
   * - A record or array value whose field or element has another length than its target's, on
       an assignment, a parameter or a qualified expression
     - Error: ``E1405`` when both lengths are static, ``E3009`` at run time otherwise. The value is
       never truncated.
     - Make both sides the same length. A design that passed on another simulator may have
       relied on the truncation: the error names the statement and both lengths.
   * - A procedure that leaves an ``out`` or ``inout`` scalar parameter with a value outside the
       subtype of the variable passed to it: 10 into a variable of ``integer range 0 to 9``
     - Error ``E3009`` at the call, as the standard says: the value copied back must belong to
       the subtype of the actual. The message ends ``for actual of parameter`` and the formal's
       name.
     - Give the variable a subtype that holds every value the procedure can return, or check the
       value in the procedure. A design that passed on another simulator may have kept the
       value outside its subtype.
   * - A procedure that reads a scalar ``out`` variable parameter before assigning it, or returns
       without assigning it
     - The parameter starts at the default value of its subtype (``'left``, ``null`` for an
       access type), as the standard says, not at the value of the variable passed to it, and
       that value is copied back. Array and record ``out`` parameters still start with the
       variable's value.
     - Assign every ``out`` parameter on every path, or make it ``inout`` if the procedure needs
       the value passed in. A design that passed on another simulator may have relied on the
       parameter keeping the variable's value.
   * - A procedure that assigns a scalar ``out`` or ``inout`` variable parameter and then reads
       the variable passed to it by its own name (a procedure declared in the same process or
       subprogram, or through a procedure, function or alias that names it)
     - The variable keeps its old value until the call returns, as the standard says: scalar and
       access parameters are passed by copy.
     - Read the parameter, not the variable, to see the new value. A design that passed on
       another simulator may have seen the new value early.
   * - A procedure that drives a value outside the subtype of the signal passed to it through an
       ``out`` or ``inout`` signal parameter: ``o <= 10`` with a signal of ``integer range 0 to 9``
     - Error ``E3009`` at the assignment in the procedure: the formal stands for the signal, so
       the value it drives must belong to the signal's subtype. The message ends ``for actual of
       signal parameter`` and the formal's name, with the signal's range. Not yet checked in a procedure
       that declares subprograms or protected types of its own.
     - Give the signal a subtype that holds every value the procedure can drive, or check the
       value in the procedure. A design that passed on another simulator may have put the value
       into the signal.
   * - A value passing through a port that does not belong to the subtype receiving it: an ``out
       integer`` port driving 10 into a signal of ``integer range 0 to 9``, an ``in natural`` port
       on an ``integer`` signal that holds -1, or a conversion in the port map whose result does
       not fit
     - Error ``E3009`` when the value passes: the message ends ``for actual of port`` (a value
       going up into the signal) or ``for port`` (a value going down into the port), with the
       port's path and the receiving range. The port's initial value counts too: an ``out
       integer`` port with no default starts at ``INTEGER'LEFT`` and fails at once on a
       ``natural`` signal. Ports whose subtypes are merely different but whose values fit run
       as before.
     - Give the port and its actual subtypes that hold every value that can pass, or a default
       in range. A design that passed on another simulator may have put the value into the
       signal, or been rejected there before it ran.
   * - A named aggregate as an operand of a predefined operator, ``c = (3 => a, 2 => b, ...)``, with
       ``c`` of a constrained array type declared ``downto``
     - The aggregate runs in the direction of the index subtype, so its leftmost element is its
       lowest index, not ``3``. ``others`` there is ``E1325``.
     - Qualify it, ``t'(3 => a, 2 => b, ...)``, to give it the type's direction.
   * - An aggregate target with a nested aggregate as an element of an array of arrays,
       ``((p, q), (r, s)) := m`` with ``m`` an array of rows
     - Error ``E1410``: each element of an aggregate target must be a name. A target of a
       two-dimensional array, and nested record targets, are accepted.
     - Name whole rows, ``(row0, row1) := m``, or assign the elements one by one.
   * - A file that declares the same design unit twice (two architectures ``a`` of ``e``)
     - Warning ``W0005``: the second replaces the first, and a unit between them that names it
       runs with the second.
     - Remove or rename one of the two.
   * - ``T'base`` as a type mark: ``integer'base'(5)``, ``new t'base'(...)``, ``x : t'base``
     - Error ``E1201`` in a VHDL-1993 or VHDL-2008 file: the standard allows ``'base`` only as
       the prefix of another attribute (``t'base'left``). A VHDL-2019 file still accepts it.
     - Name the type directly, ``integer'(5)``.
   * - A statement label with the name of a declaration of the same region: a signal and a process
       label in one architecture, a variable and a loop label in one process, a statement label
       of an entity and a signal of its architecture
     - Error ``E1101``: a label is declared in the enclosing process, subprogram, block, generate,
       architecture or entity, and two declarations in one region may not share a name. A label
       in a nested region only hides the outer name. A label with the name of an implicit
       predefined operation (``minimum : process`` next to ``type t is (u, v)``) is accepted and
       hides the operation, as LRM 2008 12.3 says, although both reference simulators reject the
       concurrent form.
     - Rename the label or the declaration.
   * - A declaration with the name ``guard`` in a guarded block: ``b : block (c)`` with a signal,
       generic, port, constant, type, subprogram or alias called ``guard``
     - Error ``E1101``: a guarded block implicitly declares the signal ``GUARD`` (LRM 2008 11.2),
       which an explicit declaration of the same name may not hide. A nested block has its own region.
     - Rename the declaration.
   * - A guard condition that reads signal attributes: ``b : block (clk'event and clk = '1')``,
       ``'last_value``, ``'stable``, ``rising_edge(clk)``
     - Accepted. GUARD starts as the condition on the values before time 0 (``'event`` FALSE,
       ``'last_event`` TIME'HIGH), with no event of its own, and keeps its value until a signal of
       the condition is active again (LRM 2008 14.7.4, 14.7.5.2), also one the condition reads only
       through ``'last_value`` or ``'last_event``, and also for a transaction that keeps the value
       (``s <= s;``). One reference simulator rejects these attributes in a guard, and the other
       evaluates the guard again only on an event.
     - Nothing; for code that must run elsewhere too, write the clocked process out.
   * - An index constraint whose discrete range has another type than its index:
       ``type nv is array (natural range <>) of bit; signal s : nv(color);`` or
       ``string_vec(severity_level)`` for an array indexed by ``natural``
     - Error ``E1300`` at the range, in every revision, whether the range is a type mark, a subtype
       indication, ``t range a to b`` or an attribute (LRM 2008 5.3.2.2: the type of each discrete
       range is the type of its index).
     - Constrain with a range of the index type, or declare the array type with the enumeration as its index.
   * - A static index out of range or a constant overflow in code that may be dead
     - Warning (``W1404``, ``W1401``); the check still runs if the code runs.
     - ``-Werror=W1404`` makes it an error.
   * - An external name as the actual of an ``in`` port
     - The port follows the signal in the same delta cycle.
     - Do not depend on the delta in which such a port changes.
   * - ``std_logic_arith``, ``std_logic_unsigned`` and the other Synopsys packages
     - Built into ``ieee`` and visible without a flag.
     - Nothing; prefer ``numeric_std`` in new code.

The complete list of choices, with the sections of the standard and portable forms, is kept in the
repository: `docs/design/language-choices.rst
<https://github.com/ru551n/yavs/blob/main/docs/design/language-choices.rst>`__.

See also
--------

* :doc:`migrating`: moving a project from another simulator.
* :doc:`language-support`: what each standard supports.
* :doc:`psl`: the subset of PSL.

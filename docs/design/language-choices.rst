Where simulators differ: the full list
======================================

Internal reference (moved off the site 2026-10-28; the user page ``docs/language-choices.rst``
keeps only the differences a user runs into, with the flag to change them). Links of the form
``:doc:`` below refer to site pages.

The VHDL standard leaves some points open, and simulators do not always agree on them. Where the
standard is clear, yavs follows it and reports a violation early. Where it is unclear or silent,
yavs is permissive and takes the reading that lets the most designs elaborate; the choices below
are the ones a design can run into. Each entry gives the construct, what yavs does, the standard
section (VHDL-2008 unless marked 2019) and, where one exists, a form that behaves the same in
every simulator. Which features of each language revision work, and how well that is checked, is
on the :doc:`language-support` page.

Elaboration and delta timing
----------------------------

External name as the actual of an ``in`` port
~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~

.. code-block:: vhdl

   m : entity work.mon port map (i => << signal .top.l.hidden : bit >>);

yavs treats the external name like a signal name: the port starts at the signal's initial value
and follows it in the same delta cycle. Some simulators treat it as an expression, so the port
starts at its own default and lags by one delta. Standard: 6.5.6.3, 8.7, 14.7.

*Portable:* do not depend on the delta in which such a port changes, or connect through an
ordinary port.

Relative external name as a port actual
~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~

.. code-block:: vhdl

   l : entity work.leaf;
   m : entity work.mon port map (i => << signal l.hidden : natural >>);

A relative pathname starts at the region that contains the port map (here the one holding ``l``),
not at the instantiated entity (``mon``). Some simulators start it at the instantiated entity and
fail to elaborate the design. Standard: 8.7.

*Portable:* name the signal with an absolute pathname or a ``^`` pathname.

Open unconstrained ``in`` port with a default
~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~

.. code-block:: vhdl

   entity sub is port (p : in bit_vector := "101"); end entity;
   ...
   u : entity work.sub port map (p => open);   -- p'length = 3

The port takes the bounds of its default. An open unconstrained ``in`` port without a default is
an error. Standard: 6.5.6.3, 14.3.5.

*Portable:* give the port an actual, or constrain it.

Record or array resolved as a whole, driven in part
~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~

.. code-block:: vhdl

   subtype rrec is res rec;     -- res resolves whole records
   signal s : rrec;
   ... s.a <= 5;                -- drives only field a

A process that drives part of a composite whose resolution function is declared on the composite
drives all of it, the other parts keeping their initial value. The resolution function sees whole
composites. Standard: 4.6, 14.7.2.

*Portable:* drive every field from every process that drives the signal.

Order of processes within a cycle
~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~

.. code-block:: vhdl

   tb  : process begin wait until clk = '1'; check(ready); wait; end process;
   dut : entity work.d port map (clk, ready);   -- its process: wait until clk = '1'; ...

The standard does not say in which order the processes resumed in one simulation cycle run
(14.7.5.3), and simulators differ. yavs fixes the order, the same at any ``--threads``: at
initialization every process runs in elaboration order. After that, the processes with one
suspension point (a sensitivity list, or exactly one wait statement and no wait in a called
procedure) run first, in elaboration order; then the other processes, in reverse elaboration
order, those resumed by a signal before those resumed by a timeout; among them the concurrent
procedure calls come before the other processes. Reports, ``textio`` output
and ``stop``/``finish`` appear in that order, and processes that share a variable see each
other's updates in it. ``--shuffle`` permutes the run order but not the order of the output.

*Portable:* do not let a result depend on which process runs first in a cycle: pass values
through signals, which every process sees one delta later.

Signal attributes at time zero
------------------------------

``'transaction`` does not toggle during initialization, and ``'delayed(T)`` shows the prefix's
initial value until ``T`` has passed. Some simulators toggle ``'transaction`` in the first delta
cycle, and some start ``'delayed(T)`` at the type's default value. Standard: 14.7.5.2.

*Portable:* ignore events of these attributes at time zero.

``'stable``, ``'quiet``, ``'delayed`` and ``'transaction`` may be named inside a subprogram body
on a signal that is not a formal parameter (the standard forbids them only on formal signal
parameters, 4.2.2.2). Some simulators reject them in any subprogram.

*Portable:* declare the implicit signal in the enclosing architecture and use that.

Literals and expressions
------------------------

Negated literal at the 64-bit minimum
~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~

.. code-block:: vhdl

   constant k : time := -9223372036854775808 fs;

The literal ``9223372036854775808`` is too large even when a minus is applied directly to it
(``E1402``), for integer and physical types: the range of ``universal_integer`` is implementation
defined (9.3, 5.2.3), and yavs stops at the 64-bit maximum (decided by the user, 2026-10-19).

*Portable:* write ``-9223372036854775807 - 1``.

TIME units below the resolution limit
-------------------------------------

With ``yavs run --resolution=ns``, any TIME literal with a unit below ``ns`` (``1 ps``, or the
bare unit ``ps``) anywhere in the elaborated design is an error before time 0, also in a package
it uses and in code that never runs. ``0 ps`` is legal, and so is ``0.4 ns``: the rule is about
the unit, not the value. Values that are not a multiple of the limit keep femtosecond precision.
Some simulators only warn and round the literal. The units of the built-in ``std`` and ``ieee``
libraries are not checked. Standard: 5.2.4.2.

*Portable:* write times in units at or above the limit you run with.

Constant errors in ranges and overflows
~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~

Division by zero in a constant range bound is an error at analysis (decided by the user,
2026-10-19). A static index or slice
outside its range, and a constant integer overflow outside a required context, are warnings
(``W1404``, ``W1401``): the code may be dead and the run-time check stays. Some simulators make
these errors, others defer them to elaboration. Decided by the user, 2026-10-19. Standard: 9.4.2,
8.4, 8.5.

Condition operator from the type's region
~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~

.. code-block:: vhdl

   use ieee.std_logic_1164.std_ulogic;   -- only the type is visible
   ...
   if s then ...                         -- uses the "??" declared with the type

When no ``??`` is visible, the one declared in the region of the condition's type is used.
Two different visible ``??`` for the type is an error. Some simulators reject the first form.
Standard: 9.2.9. Decided by the user, 2026-10-19.

*Portable:* ``use ieee.std_logic_1164.all`` or write ``s = '1'``.

Aggregate targets with range or element choices
~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~

.. code-block:: vhdl

   (5 downto 2 => hi, 1 downto 0 => lo) := v;

Accepted. The standard does not say how the choices map to the value's elements, and one
simulator rejects the form. yavs gives the aggregate the bounds of its choices, in the
direction of its range choices, and matches its leftmost index with the value's leftmost
element: ``(0 to 1 => a, 2 to 5 => b) := v`` gives ``a`` the two leftmost elements of ``v``,
whatever ``v``'s bounds. Associations may be written in any order. Each target must have as
many elements as its choice. Element choices alone work the same way: with ``v : iv(1 downto 0)``,
``(0 => a, 1 => b) := v;`` gives ``a`` the value ``v(1)``, the leftmost element, not ``v(0)``. Without a
range choice the aggregate runs in the direction of the array type's index subtype.
Standard: 10.5.2.1, 10.6.2.1, 9.3.3.3.

*Portable:* assign slices or elements: ``hi := v(5 downto 2); lo := v(1 downto 0);``, ``a := v(1); b := v(0);``.

Array values under range choices in aggregates
~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~

.. code-block:: vhdl

   f((3 downto 2 => a, 1 downto 0 => b));   -- f takes an unconstrained bit_vector
   v := (1 downto 0 => a, 2 to 3 => b);     -- E1334

Without a constrained context, such an aggregate takes the direction of its range choices
(here ``3 downto 0``), so ``a`` fills elements 3 and 2. Choices of different directions are
rejected at analysis (``E1334``). Against the direction of a constrained target the choices are
accepted and each range is filled in the target's order; some simulators reject that.
Decided by the user, 2026-10-19. Standard: 9.3.3.3.

*Portable:* give every such choice the direction of the target, or qualify the aggregate with a
constrained subtype.

Generics, packages and subprograms
----------------------------------

Positional generic map after a generic type
~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~

.. code-block:: vhdl

   entity sub is generic (type t; width : natural); ...
   u : entity work.sub generic map (integer, 8);

The implicit ``"="`` and ``"/="`` of the interface type take no position, so ``8`` is ``width``.
Some simulators count them and reject the map. Standard: 6.5.3, 6.5.7.2.

*Portable:* use named association: ``generic map (t => integer, width => 8)``.

Generic subprograms of components and blocks
~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~

A component's generic type, subprogram and package are bound to the entity's by name. A generic
subprogram of an enclosing entity may be passed down as the actual of a nested instance's generic
subprogram (some simulators reject this). A generic subprogram of a *block* or a *component*
used that way is an error (``E1506``), as in all other simulators (decided by the user,
2026-10-19). Standard: 6.5.7.2, 7.3.3.

*Portable:* pass a subprogram declared in the architecture or a package.

Package instances in processes, subprograms and generates
~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~

Package instantiations are accepted in any declarative part (4.3, 11.3, 11.8), including
processes, subprograms and generate statements; each entity instance gets its own package
objects. Some simulators reject them there.

*Portable:* instantiate in the architecture or in a package.

Types reached through a formal package of a formal package
~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~

A type written through two levels of formal packages (``s.r.arr``) is the type of the actual
instance, so a call in an instance accepts an actual of that type (6.5.7.2, 4.8; decided by the
user, 2026-10-19). Most other simulators reject it. *Portable:* pass the inner package as a
generic of its own.

Subprogram header with a generic map
~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~

.. code-block:: vhdl

   function f generic (type t) generic map (t => integer) parameter (v : t) return t is ...

The header declares an instance of the subprogram, called with the instantiated profile
(2019, 4.2.1, 4.4). Other simulators treat the subprogram as uninstantiated and reject calls.

*Portable:* ``function f_int is new f generic map (t => integer);``.

Record with protected and other elements
~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~

.. code-block:: vhdl

   type rec is record a : counter_t; n : integer; end record;   -- counter_t protected

VHDL-2019 allows arrays and records of protected types, but a record that mixes a protected
element with elements of other types is rejected (``E1905``, 2019 5.3.1): if one element is
protected, all must be. Each protected element is its own object; the record cannot be assigned,
compared or, unless every scalar subelement is protected, be a shared variable.

*Portable:* keep the other elements out of the record, or inside a protected type.

Interface lists
~~~~~~~~~~~~~~~

A type, subprogram or package in a *parameter* list (4.2.2.1), an incomplete type that is never
completed (5.4.2), two parameters with one name (12.3) and a use-clause clash between a constant
and an enumeration literal (12.4) are errors in yavs; some simulators accept them.

Subprogram calls
----------------

Conversions on unconstrained ports
~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~

.. code-block:: vhdl

   u1 : entity work.sub port map (i => w4(u));    -- subtype w4 is bit_vector(1 to 4)
   u2 : entity work.sub port map (u4(o) => s);    -- subtype u4 is unsigned(7 downto 4)

An unconstrained port takes its index ranges from the conversion (5.3.2.2): the type mark or the
function's result subtype in the actual part, the type mark or the function's parameter subtype
in the formal part. A conversion that does not define them, ``i => bit_vector(u)``, is an error
(``E1312``; decided by the user, 2026-10-19). Some simulators give the port the actual's bounds
instead, or reject these forms.

*Portable:* constrain the port, or convert into a signal of the wanted subtype.

Conversions in call associations
~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~

.. code-block:: vhdl

   seti(real(x) => r);      -- formal conversion of an out parameter
   geti(integer(r));        -- actual conversion of an in variable parameter

Formal conversions are accepted for ``out`` and ``inout`` variable parameters, actual conversions
only where data flows in. A formal that is not fully constrained takes its index ranges from its conversion, as for ports
(5.3.2.2): ``bv4(x) => v`` with a constrained subtype ``bv4``, or a conversion function whose
parameter subtype is constrained. A conversion that defines no bounds, ``bit_vector(x) => v``,
is an error (``E1312``).
Some simulators do not support conversions in calls (6.5.7.1).

*Portable:* convert into a temporary variable before and after the call.

Named return values (2019)
~~~~~~~~~~~~~~~~~~~~~~~~~~

A call of a function declared ``return r of t`` takes its result subtype from the context: an
assignment target, an initial value, a ``return`` statement, a constrained formal or a qualified
expression; elsewhere it is an error (``E1304``). Some simulators accept fewer contexts
(4.2.1, 2019).

``t`` may be a scalar type (integer, enumeration, physical, floating point, a subtype with a range
constraint, a generic type whose actual is scalar): ``r`` is then the target's subtype, so
``r'low``, ``r'high``, a variable of subtype ``r`` and its range checks follow the target's range,
and the returned value is checked against ``t``. Some simulators reject scalar named return
values, and some give ``r`` the range of ``t`` instead of the target's (4.2.1, 2019). A named
return value of an access type is an error (``E1233``; decided by the user, 2026-10-19).

*Portable:* pass the range as a parameter, or return a value of ``t`` and let the assignment
check it.

Access and file types (2019)
~~~~~~~~~~~~~~~~~~~~~~~~~~~~

In 2008 files, constants, signals, ports and constant parameters of an access or file type, and
protected method parameters and results of such types, are errors (``E1223``). In 2019 files
constant parameters of access type are errors too (6.5.2), while protected method parameters and
results are accepted (decided by the user, 2026-10-19), and so are access types designating
protected or file types (5.4.1, 5.6.2, 6.4.2.2).

*Portable:* use variable parameters.

Aliases of protected methods (2019)
~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~

.. code-block:: vhdl

   alias push is sv.push [integer];    -- sv is a shared or process variable

A method of a shared variable or process variable can be aliased and called through the alias
(5.6.2, 6.6.3). Some simulators fail on the call.

*Portable:* call ``sv.push(x)``.

``unaffected`` in assignments (2019)
~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~

``v := unaffected;``, ``w := 3 when c else unaffected;`` and ``s <= force unaffected;`` leave the
target unchanged; in 2008 files they are errors (10.5.2.1).

Configurations
--------------

A block configuration of a default-bound instance is accepted: the default binding supplies
the entity (3.4.3, 7.3.3; decided by the user, 2026-10-19). A configuration that
names an instance of another component, or binds an instance that a configuration
specification already binds to a different entity aspect, is an error at analysis (7.3); an
incremental binding may repeat the entity aspect of the primary binding (7.3.2.1). Some simulators
check these only at elaboration.

VHDL-1993 mode
--------------

``--std=93`` runs 1993 code with the 1993 packages. These points are choices, not requirements of the
1993 standard:

* Standards may be mixed. A design of files of several standards is elaborated with the packages of
  the newest one, where other simulators refuse to mix. A 1993 file in such a design is checked as a
  1993 file but sees the newer packages, so 1993 code that overloads a subprogram on both
  ``std_logic_vector`` and ``std_ulogic_vector`` fails there with a duplicate declaration (``E1101``).
  *Portable:* keep such code in a design of 1993 files alone.
* An explicit declaration hides an implicit one when both are visible through use clauses, as in 2008.
  Other simulators need a switch for this in 1993 mode.
* The 1993 restrictions that 2008 relaxed are errors in 93 mode (``E1007``, naming the 1993 rule):
  an alias of a multidimensional array, an unconstrained element type, a ``buffer`` port associated
  with an ``out`` or ``inout`` port, a package inside a declarative part and an array value as a part
  of an aggregate. Some other relaxed static and ``case`` rules are still accepted.
* 2008 syntax that is cheap to recognise is rejected: ``process (all)``, conditional assignments in
  a process, ``elsif`` and ``case`` generate and package generics are errors (``E0011``).

``--relaxed``
-------------

``--relaxed`` turns some errors into warnings of their own: a shared variable of a type that is not
protected (``W1904``), a pure function that reads or calls something impure (``W1830``) and an
``others`` aggregate assigned to an object with an unconstrained subtype, which then takes the
object's bounds at run time (``W1325``). ``run`` and ``elab`` analyse the files again, so give the
option to them as well as to ``analyze``.

PSL
---

yavs runs a subset of PSL (see ``docs/language-support.md``) and reads it in ``-- psl`` comments only with
the option ``--psl``, as the simulators that read PSL in comments do; some simulators reject PSL
altogether. A directive is checked in the delta cycle of its clock event, with the values the
signals have then. yavs also accepts an ``@`` clock on a directive, ``assume`` and a ``severity``
clause on an assertion, which some simulators reject. The default failure message is ``PSL
assertion failed``; simulators word it differently. PSL outside the subset is skipped with
warning ``W0004`` instead of failing the design.

*Portable:* use a ``default clock`` declaration and ``assert`` directives with a ``report``
clause.

Library
-------

The non-standard Synopsys packages ``std_logic_arith``, ``std_logic_unsigned``,
``std_logic_signed`` and ``std_logic_misc`` are built into library ``ieee`` as the original
files and are visible without a flag, so existing designs that use them analyze and simulate.
New code should prefer ``numeric_std``.

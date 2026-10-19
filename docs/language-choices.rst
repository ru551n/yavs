Where simulators differ
=======================

The VHDL standard leaves some points open, and simulators do not always agree on them. Where the
standard is clear, yavs follows it and reports a violation early. Where it is unclear or silent,
yavs is permissive and takes the reading that lets the most designs elaborate; the choices below
are the ones a design can run into. Each entry gives the construct, what yavs does, the standard
section (VHDL-2008 unless marked 2019) and, where one exists, a form that behaves the same in
every simulator.

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

The minus applies to the abstract literal, so ``-9223372036854775808`` is accepted for integer and
physical types; ``9223372036854775808`` without the minus is an error. Many simulators reject the
literal. Standard: 9.3, 5.2.3 (the range of ``universal_integer`` is implementation defined).

*Portable:* write ``-9223372036854775807 - 1``.

Constant errors in ranges and overflows
~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~

Division by zero in a constant range bound is an error at analysis. A static index or slice
outside its range, and a constant integer overflow outside a required context, are warnings
(``W1404``, ``W1401``): the code may be dead and the run-time check stays. Some simulators make
these errors, others defer them to elaboration. Standard: 9.4.2, 8.4, 8.5.

Condition operator from the type's region
~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~

.. code-block:: vhdl

   use ieee.std_logic_1164.std_ulogic;   -- only the type is visible
   ...
   if s then ...                         -- uses the "??" declared with the type

When no ``??`` is visible, the one declared in the region of the condition's type is used.
Two different visible ``??`` for the type is an error. Some simulators reject the first form.
Standard: 9.2.9.

*Portable:* ``use ieee.std_logic_1164.all`` or write ``s = '1'``.

Aggregate targets with range choices
~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~

.. code-block:: vhdl

   (5 downto 2 => hi, 1 downto 0 => lo) := v;

Rejected at analysis (``E1333``): the standard does not say how the choices map to the value's
elements, and simulators disagree on whether it is legal. Standard: 10.5.2.1.

*Portable:* assign slices: ``hi := v(5 downto 2); lo := v(1 downto 0);``.

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
used that way is an error (``E1506``), as in all other simulators. Standard: 6.5.7.2, 7.3.3.

*Portable:* pass a subprogram declared in the architecture or a package.

Package instances in processes, subprograms and generates
~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~

Package instantiations are accepted in any declarative part (4.3, 11.3, 11.8), including
processes, subprograms and generate statements; each entity instance gets its own package
objects. Some simulators reject them there.

*Portable:* instantiate in the architecture or in a package.

Types reached through a formal package of a formal package
~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~

A type written through two levels of formal packages (``s.r.arr``) is rejected, as most
simulators do. *Portable:* pass the inner package as a generic of its own.

Subprogram header with a generic map
~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~

.. code-block:: vhdl

   function f generic (type t) generic map (t => integer) parameter (v : t) return t is ...

The header declares an instance of the subprogram, called with the instantiated profile
(2019, 4.2.1, 4.4). Other simulators treat the subprogram as uninstantiated and reject calls.

*Portable:* ``function f_int is new f generic map (t => integer);``.

Interface lists
~~~~~~~~~~~~~~~

A type, subprogram or package in a *parameter* list (4.2.2.1), an incomplete type that is never
completed (5.4.2), two parameters with one name (12.3) and a use-clause clash between a constant
and an enumeration literal (12.4) are errors in yavs; some simulators accept them.

Subprogram calls
----------------

Conversions in call associations
~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~

.. code-block:: vhdl

   seti(real(x) => r);      -- formal conversion of an out parameter
   geti(integer(r));        -- actual conversion of an in variable parameter

Formal conversions are accepted for ``out`` and ``inout`` variable parameters, actual conversions
only where data flows in. A conversion on an unconstrained formal is an error (``E1312``).
Some simulators do not support conversions in calls (6.5.7.1).

*Portable:* convert into a temporary variable before and after the call.

Named return values (2019)
~~~~~~~~~~~~~~~~~~~~~~~~~~

A call of a function declared ``return r of t`` takes its result subtype from the context: an
assignment target, an initial value, a ``return`` statement, a constrained formal or a qualified
expression; elsewhere it is an error (``E1304``). Some simulators accept fewer contexts
(4.2.1, 2019).

Access and file types (2019)
~~~~~~~~~~~~~~~~~~~~~~~~~~~~

In 2008 files, constants, signals, ports and constant parameters of an access or file type, and
protected method parameters and results of such types, are errors (``E1223``). In 2019 files
constant parameters of access type and protected method parameters and results are accepted,
and so are access types designating protected or file types (5.4.1, 5.6.2, 6.4.2.2).
Some simulators reject them in both revisions.

*Portable:* use variable parameters in 2008 style.

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

A block configuration of a default-bound instance is accepted (3.4.3). A configuration that
names an instance of another component, or re-binds an instance that a configuration
specification already binds, is an error at analysis (7.3); some simulators check these only at
elaboration.

Library
-------

The non-standard arithmetic packages ``std_logic_arith`` and ``std_logic_unsigned`` are not part
of the standard libraries that ship with yavs (``E1002``); use ``numeric_std``.

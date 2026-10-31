.. This page is generated from the tables behind ``yavs --explain`` (crates/yavs/src/reference.rs).
.. Do not edit it. After a change to a code or its explanation regenerate it with:
..
..     YAVS_BLESS=1 cargo test -p yavs --bin yavs cli_reference

Error codes
===========

Every diagnostic code of yavs with its meaning, for looking up a code you got: ``E`` is an error,
``W`` a warning and ``L`` a library error. ``yavs --explain CODE`` prints the whole explanation.
The search box of this site finds a code, and a link to one ends in ``#e1100``.

.. contents:: On this page
   :local:

Syntax, command line, waveforms, coverage, debugger, plugins (E0 and W0)
------------------------------------------------------------------------

.. list-table::
   :header-rows: 1
   :widths: 12 88

   * - Code
     - Meaning
   * - _`E0001`
     - The lexer found an invalid character, an unterminated string, character literal or comment, or a malformed number.
   * - _`W0001`
     - A ```warning`` conditional analysis directive was reached; the message is the one in the directive.
   * - _`E0002`
     - A token the grammar does not allow at this point; the label says what was expected.
   * - _`W0002`
     - A tool directive that yavs does not know was ignored.
   * - _`E0003`
     - The name after ``end`` does not repeat the name of the construct it closes.
   * - _`W0003`
     - A number is directly followed by an identifier, as in ``10ns``; write a space between them.
   * - _`E0004`
     - A VHDL-2019 construct was used in a VHDL-2008 file.
   * - _`W0004`
     - A design unit contains PSL (IEEE 1850) outside the subset yavs runs.
   * - _`E0006`
     - Constructs are nested deeper than the parser's limit.
   * - _`E0007`
     - A conditional analysis directive (```if``, ```elsif``, ```else``, ```end``) is malformed or unbalanced.
   * - _`E0008`
     - A ```error`` conditional analysis directive was reached; the message is the one in the directive.
   * - _`E0009`
     - A literal or identifier contains a character outside ISO-8859-1, which VHDL does not allow.
   * - _`E0010`
     - The file is not valid UTF-8 and ``--encoding=utf8`` was given.
   * - _`E0011`
     - A VHDL-2008 construct was used in a VHDL-1993 file.
   * - _`E0012`
     - The file holds an encrypted (IEEE 1735 protected) envelope, ```protect begin_protected`` or ```pragma protect begin_protected``.
   * - _`E0100`
     - A source file named on the command line cannot be read.
   * - _`E0101`
     - A response file (``@FILE``) cannot be read or is not valid UTF-8.
   * - _`E0102`
     - ``std`` and ``ieee`` are provided by yavs and cannot be mapped with ``--work`` or ``--map`` (except ``yavs analyze --bootstrap``).
   * - _`E0103`
     - An unsupported ``-W`` option.
   * - _`E0104`
     - The top level named on the command line (``ENTITY``, ``ENTITY.ARCH`` or a configuration, with no library prefix) is not in the work library.
   * - _`E0105`
     - ``--stop-time`` is not a valid time such as ``10ms``.
   * - _`E0106`
     - A ``--load`` library cannot be loaded: the file is missing, has the wrong architecture, or one of its own dependencies cannot be found.
   * - _`E0107`
     - A foreign subprogram cannot be bound or called: its VHPIDIRECT C signature cannot be called, its VHPI model is registered as the other kind of subprogram, or (at the call) no ``--load`` library and not the executable defines its symbol or no ``--vhpi`` plugin registered its model.
   * - _`W0108`
     - A VHPIDIRECT foreign subprogram's symbol is defined in no ``--load`` library and not in the executable, or a ``"VHPI library model"`` subprogram's model was registered by no ``--vhpi`` plugin.
   * - _`W0109`
     - ``--threads`` is ignored with ``--dap``, ``--interactive`` and ``--do``: a debug session stops inside processes, which needs the simulation on one thread.
   * - _`E0110`
     - A coverage database or report cannot be written (``yavs run --cov=FILE``, ``yavs cov merge -o FILE``, ``yavs cov report --cobertura FILE``): the note gives the operating system's reason.
   * - _`W0110`
     - A row of the exclusion file names a line (``file:line``) that is not where the excluded statement is now.
   * - _`E0111`
     - A coverage database given to ``yavs cov merge`` or ``yavs cov report`` cannot be read, or is not a database ``yavs run --cov`` wrote: the note says which.
   * - _`W0111`
     - A row of the exclusion file matches nothing in the coverage database: the statement changed (its text hash with it), was removed, or the instance path is wrong.
   * - _`E0112`
     - An exclusion file given to ``yavs cov report --exclude`` cannot be read or has a malformed row or entry (a ``.yaml`` or ``.yml`` file is read as YAML, any other as text rows); the note names the line.
   * - _`W0112`
     - A ``-- coverage off`` pragma has no ``-- coverage on`` after it, so it excludes the rest of the file from coverage.
   * - _`E0113`
     - A source file named by the coverage database cannot be read, so ``yavs cov report --html DIR`` cannot show it; the note gives the reason.
   * - _`W0113`
     - A ``-- coverage on`` pragma has no ``-- coverage off`` before it.
   * - _`E0114`
     - The total coverage of ``yavs cov report`` is below the percentage of ``--fail-under=PCT``.
   * - _`W0114`
     - A row of the exclusion file excludes an item that ran in the merged runs (for a ``toggle`` row: the scalar toggled both ways).
   * - _`E0115`
     - ``yavs cov report --total`` or ``--fail-under`` has no item to count: the coverage database holds none of the kinds selected with ``--kind`` (or nothing is left after the exclusions).
   * - _`W0115`
     - A signal or port has more ``bit``/``std_ulogic`` scalars than ``--cov-toggle-max`` (default 65536), so its toggles are not counted.
   * - _`W0116`
     - The signal handlers of a running simulation (Ctrl-C and SIGTERM writing out the buffered output, SIGUSR1 printing a hang report) could not be installed (the note gives the operating system error).
   * - _`W0117`
     - The compile thread of ``--tier=auto`` could not be started (the note gives the operating system error).
   * - _`W0118`
     - ``--threads`` is above 1 and the token server named in ``YAVS_TOKENS`` cannot be reached (nothing listens on that port of 127.0.0.1, it does not answer, or the connection broke during the run; the note says which).
   * - _`E0120`
     - ``yavs run --tier=...`` names a compiled tier that this yavs was built without.
   * - _`E0121`
     - The LLVM tier could not compile the design for ``--tier=llvm``, or could not start compiling for ``--tier=auto``.
   * - _`E0123`
     - ``yavs cache clean`` could not remove a compilation cache entry of a library directory (permissions, or the directory is in use by a read-only mount).
   * - _`E0124`
     - ``yavs run --profile=FILE`` could not write the profile file (missing directory, permissions, full disk).
   * - _`E0125`
     - ``yavs run --profile`` was combined with ``--dap``.
   * - _`E0126`
     - The directory of the std/ieee units shared by all libraries cannot be used: ``--std-cache`` or ``YAVS_STD_CACHE`` is empty, no user cache directory is known, or (for ``yavs analyze``) the directory cannot be created or written.
   * - _`E0127`
     - ``yavs run --tier=llvm`` could not load the LLVM 21 shared library (Linux).
   * - _`E0128`
     - ``--threads`` is above 1 and ``YAVS_TOKENS`` names a token server that yavs cannot use: the value is not ``<port>:<secret>``, or the server did not accept the secret (often a ``YAVS_TOKENS`` left over from a test runner that has ended).
   * - _`E0130`
     - ``--surfer`` cannot use Surfer: ``surfer`` is not on PATH, it ended or did not connect within 30 seconds, the address does not answer, it refuses the greeting, or it does not load the file.
   * - _`W0131`
     - The connection to Surfer failed during the run, or Surfer stopped answering for 30 seconds.
   * - _`E0133`
     - ``yavs lint`` was given nothing to check, a path that does not exist, or a rule that does not exist: name files or directories, or give ``--work`` with an analyzed library.
   * - _`E0134`
     - ``yavs lint`` cannot use its configuration or waiver file: it cannot be read, is not valid YAML, or names a key, class or rule that does not exist.
   * - _`E0135`
     - ``yavs lint`` cannot write a report file, named by ``--json``, ``--sarif``, ``--junit``, ``--quality-report`` or ``--sonarqube``.
   * - _`E0199`
     - A command option or combination is not implemented yet.
   * - _`E0701`
     - The Debug Adapter Protocol connection (``--dap``) cannot be opened: the port is in use or not allowed, or accepting the client failed.
   * - _`E0702`
     - The live waves of a ``yavs/waveSubscribe`` subscription failed while the simulation ran (a value the recorder cannot encode); the note says why.
   * - _`E0703`
     - The console hook named in ``YAVS_CONSOLE_HOOK`` cannot be used: the value is not ``<port>:<secret>``, nothing answers on that port within 30 seconds, or what it registers is not valid (a yavs command's name).
   * - _`E0801`
     - The waveform file given to ``--wave`` cannot be created or written (missing directory, permissions, full disk).
   * - _`W0802`
     - An array or vector has more elements than ``--wave-max-array`` and is left out of the waveform.
   * - _`E0803`
     - The Ctrl-C handler for ``--wave`` cannot be installed.
   * - _`E0901`
     - A VHPI plugin (``--vhpi``) cannot be loaded: the file is missing, has the wrong architecture, or imports a ``vhpi_*`` symbol yavs does not export.
   * - _`E0902`
     - A VHPI plugin has neither the named entry point nor a ``vhpi_startup_routines`` array.
   * - _`W0903`
     - A VHPI call was misused (a bad handle, property or argument); the call failed.
   * - _`W0904`
     - A VHPI call that yavs does not support yet.
   * - _`E0905`
     - An architecture's ``foreign`` attribute names a VHPI model (``"VHPI library model"``) that no plugin given with ``--vhpi`` registered with ``vhpi_register_foreignf`` (``vhpiArchF``); the architecture would have no behaviour.

Libraries (L0)
--------------

.. list-table::
   :header-rows: 1
   :widths: 12 88

   * - Code
     - Meaning
   * - _`L0000`
     - A note from the library layer, such as a lock that is being waited for.
   * - _`L0001`
     - A mapped library does not exist.
   * - _`L0002`
     - A library manifest or record cannot be read, written or resolved.
   * - _`L0003`
     - A library manifest (the index ``yavs-lib.toml``) or a record of a file in ``yavs-lib.d/`` is corrupt, damaged or missing.
   * - _`L0004`
     - A library manifest has a format version this yavs does not read, or was written before the rename to yavs.
   * - _`L0010`
     - A library directory cannot be created or is not writable.
   * - _`L0011`
     - The library lock file could not be taken.
   * - _`L0020`
     - A library mapping is not of the form ``NAME:PATH``.
   * - _`L0030`
     - A unit refers to a library that is not mapped.
   * - _`L0031`
     - A design unit is not in the library it was looked up in.
   * - _`L0032`
     - Design units depend on each other in a cycle.
   * - _`L0033`
     - A source file recorded in a library manifest cannot be read.
   * - _`L0034`
     - A source file changed on disk since it was analyzed into the library.

Semantic analysis (E1 and W1)
-----------------------------

.. list-table::
   :header-rows: 1
   :widths: 12 88

   * - Code
     - Meaning
   * - _`E1001`
     - A library clause or selected name names a library that is not mapped.
   * - _`E1002`
     - A design unit was not found in the library it was looked up in.
   * - _`E1003`
     - Design units depend on each other in a cycle, so none of them can be analyzed first.
   * - _`E1004`
     - A secondary unit (architecture or package body) names a primary unit that does not exist.
   * - _`E1005`
     - A context declaration may not name the library ``work`` (LRM 13.4).
   * - _`E1007`
     - A declaration or construct is not available under the revision the file is analyzed with, or means something the design revision does not allow.
   * - _`E1008`
     - A name denotes a design unit of the wrong kind here, such as a package where an entity is needed.
   * - _`W1008`
     - The construct means something different under the design's VHDL-2019 std/ieee than under VHDL-2008, for example ``integer'high``.
   * - _`E1009`
     - A source file named by a library manifest or the std/ieee tree cannot be read.
   * - _`E1100`
     - The name is not declared, or not visible, at this point.
   * - _`E1101`
     - A declaration with the same designator (and, for subprograms, the same parameter and result type profile) already exists in this declarative region.
   * - _`E1102`
     - Two use clauses make different declarations with this name potentially visible, so neither is visible (LRM 12.4).
   * - _`E1103`
     - The named entity exists but is hidden at this point.
   * - _`E1104`
     - The prefix of a use clause, context reference or expanded name must denote a library, a package, a package instance or a context declaration (LRM 12.4, 13.4).
   * - _`E1105`
     - The declaration is not in the visible part of its package or protected type.
   * - _`E1106`
     - The name after ``end`` does not repeat the name of the construct.
   * - _`E1201`
     - A type mark must denote a type or subtype.
   * - _`E1210`
     - An index or element constraint is applied to a dimension that is already constrained.
   * - _`E1211`
     - This kind of constraint does not apply to the type (a range constraint on a composite, an index constraint on a scalar, ...).
   * - _`E1212`
     - Signals and variables need a fully constrained subtype.
   * - _`E1213`
     - A record constraint names an element that does not exist, or names one twice.
   * - _`E1214`
     - An index constraint has a different number of ranges than the array has dimensions.
   * - _`E1215`
     - A range or index constraint with static bounds must lie inside the range of its type mark or index subtype (LRM 5.2.1, 5.3.2.2, 6.3); a null range is always compatible.
   * - _`E1220`
     - An incomplete type declaration is not completed in the same declarative region, or the incomplete type is used before its full declaration other than as the designated subtype of an access type (LRM 5.4.2).
   * - _`E1221`
     - In VHDL-2008 the designated type of an access type may not be a file type or a protected type (LRM 5.4.1); VHDL-2019 allows both.
   * - _`E1222`
     - The element type of a file type may not be an access, protected or file type, or contain one.
   * - _`E1223`
     - A constant, signal, generic, port or constant or signal parameter may not be of an access or file type, or contain one (LRM 6.4.2.2, 6.4.2.3, 6.5.2): only variables hold access values (also in VHDL-2019; only library ``std`` declares such parameters).
   * - _`E1224`
     - A constant without a value is a deferred constant, which only a package declaration may hold (LRM 6.4.2.2); anywhere else the constant needs an initial value.
   * - _`E1225`
     - A resolution function must be a pure function with one constant parameter of a one-dimensional unconstrained array of the resolved type, returning that type.
   * - _`E1226`
     - A deferred constant is named before the end of its full declaration (LRM 6.4.2.2): in an initial value of its package, or anywhere in its package body (subprogram bodies included) before the full declaration.
   * - _`E1227`
     - The subtype indication of the full declaration of a deferred constant does not conform to the deferred declaration's (LRM 6.4.2.2): use the same type mark and constraint.
   * - _`E1230`
     - A subprogram or protected type is declared but its body is missing, or a deferred constant of a package has no full declaration in the package body (LRM 4.8).
   * - _`E1231`
     - The type definition is not legal, for example a range whose bounds are not of an integer or floating point type.
   * - _`E1232`
     - A subprogram's parameter list declares only interface constants, signals, variables and files (LRM 4.2.2.1).
   * - _`E1233`
     - The type mark after the return identifier of a VHDL-2019 named return value (``return r of t``) denotes a scalar, array or record type: ``r`` takes the range or bounds of the call's target, which an access or file type does not have (a generic type is checked in each instance).
   * - _`E1234`
     - A port clause declares only interface signals (LRM 6.5.6.3).
   * - _`E1235`
     - A formal generic subprogram has no generic list or generic map of its own (LRM 6.5.4): ``generic (function f (x : t) return t)`` takes its profile from its parameters and return type alone.
   * - _`E1236`
     - A subprogram parameter has mode ``in``, ``out`` or ``inout`` (LRM 4.2.2.1); ``buffer`` and ``linkage`` are port modes.
   * - _`E1237`
     - A process or subprogram declares no signals and no shared variables (LRM 11.3, 4.3), also not in a package declared there or through a package instance there (LRM 4.7, 4.9): such objects belong to the concurrent region around it.
   * - _`E1238`
     - A variable declared in an entity, architecture, block, generate or package is a shared variable and needs ``shared`` (LRM 6.4.2.4); a plain ``variable`` belongs in a process, subprogram or protected type body.
   * - _`E1239`
     - The declarative part does not list this item.
   * - _`E1240`
     - A subprogram body's specification does not conform to the declaration it completes (LRM 4.10): both must be the same sequence of lexical elements, apart from a numeric literal written with another value spelling and a simple name written as an expanded name.
   * - _`E1300`
     - The expression has a different type than the context requires.
   * - _`E1301`
     - None of the visible subprograms or literals with this name accepts these operands.
   * - _`E1302`
     - More than one interpretation fits; qualify an operand or use a selected name.
   * - _`E1303`
     - An operator symbol that is not a VHDL operator, or an operator function with the wrong number of parameters (LRM 4.5.2).
   * - _`E1304`
     - A function with a named return value (``return r of t``, VHDL-2019) takes its result subtype from the context of the call: the call must be the value of an assignment, initial value or ``return``, an actual of a constrained formal or a qualified expression, of a fully constrained subtype.
   * - _`E1310`
     - A formal is associated more than once.
   * - _`E1311`
     - A formal without a default has no actual.
   * - _`E1312`
     - A conversion in the association of a formal that is not fully constrained must give the formal its bounds (5.3.2.2): in the formal part the parameter subtype of the function or the type mark of the type conversion, in the actual part the result subtype of the function or the type mark, must be constrained.
   * - _`E1313`
     - A conversion in an association that the formal does not allow (6.5.7.1, 4.2.2.3): a formal conversion only for ``out`` and ``inout`` variable parameters (and ports other than ``in``), an actual conversion only where data flows in, an ``inout`` parameter converts both ways or not at all, and a signal parameter takes no conversion.
   * - _`E1320`
     - The type of an aggregate comes only from its context, and this context does not give one, or gives a type that is not an array or record type.
   * - _`E1321`
     - A record aggregate does not give a value for every element.
   * - _`E1322`
     - A record aggregate gives a value for an element twice, or has more positional elements than the record has elements.
   * - _`E1323`
     - An array aggregate element is neither of the element type nor of the aggregate type, an element of a multidimensional aggregate is not an aggregate (or string literal) for the next dimension, or one value is given for record elements of different types.
   * - _`E1325`
     - ``others`` must be last and alone, and needs a constrained context subtype.
   * - _`W1325`
     - ``--relaxed`` (R5): an aggregate with ``others`` is assigned to an object whose subtype is unconstrained (a port, a formal); it takes the bounds of the object at run time.
   * - _`E1326`
     - A choice that is not locally static, or a null range choice, must be the only choice of the only association, and named and positional associations do not mix (only a final ``others`` may follow positional ones).
   * - _`E1327`
     - Choices overlap or repeat a value.
   * - _`E1328`
     - Choices do not cover every value of the subtype and there is no ``others``.
   * - _`E1329`
     - A record aggregate choice must be an element name, not an expression or a range.
   * - _`E1330`
     - A character of a string literal is not a literal of the element type.
   * - _`E1331`
     - Adjusting the length of a bit string literal would drop significant digits (non-zero for unsigned, differing from the sign for signed), or an empty signed bit string cannot be extended.
   * - _`E1332`
     - A literal is malformed in a way the lexer accepts: a digit outside its base, a base outside 2 to 16, a negative exponent in an integer literal, a non-digit in a decimal bit string, or a physical literal with a unit the type does not have.
   * - _`E1334`
     - The range choices of array values in one aggregate (``(0 to 1 => a, 3 downto 2 => b)``) have different directions: without a constrained context the aggregate takes the direction of these choices.
   * - _`E1335`
     - An aggregate target identifies one variable or signal (or a part of it) more than once: by one association with several choices, or by two elements naming the same object (LRM 10.5.2.1, 10.6.2.1).
   * - _`E1340`
     - A condition must be BOOLEAN, or of a type with a visible ``??`` operator.
   * - _`E1341`
     - More than one ``??`` operator applies to this condition.
   * - _`E1350`
     - A type conversion is only legal between closely related types.
   * - _`E1360`
     - The object's class or mode does not allow this use (assigning an ``in`` port, reading a ``linkage`` port, reading an ``out`` port or parameter in a VHDL-1993 file, a variable assignment to a signal, ...).
   * - _`E1361`
     - The name does not denote an object or value here.
   * - _`W1361`
     - ``--relaxed`` (R6): the actual of a signal parameter is not a static name (an index by a variable or loop parameter); LRM 4.2.2.3 requires one, the call passes the element the index selects at the call.
   * - _`E1362`
     - The target of a force or release assignment is a member (an element, slice or field) of a signal whose composite subtype is resolved as a whole (LRM 10.5.2.1); force or release the whole composite signal.
   * - _`E1401`
     - Static integer or physical arithmetic overflows 64 bits.
   * - _`W1401`
     - Static arithmetic overflows in a context that does not require the value; the runtime check stays.
   * - _`E1402`
     - A literal is too large to represent.
   * - _`E1403`
     - A static value lies outside the static subtype it is converted to.
   * - _`W1403`
     - A static operand of a qualified expression or type conversion lies outside the subtype it is converted to, or an array with static bounds converted to an unconstrained array type has bounds outside the target's index subtype; the code may be dead, so this is checked again at runtime.
   * - _`W1404`
     - A static index is outside the static range of the array; the code may be dead, so this is checked again at runtime.
   * - _`E1405`
     - The static lengths of the two sides differ.
   * - _`E1410`
     - The expression must be locally static here.
   * - _`E1501`
     - The named unit is not a generic unit.
   * - _`W1501`
     - A generic is never used.
   * - _`E1502`
     - A generic without a default has no actual.
   * - _`E1503`
     - The actual does not belong to the class of the formal generic type.
   * - _`E1504`
     - No unique subprogram matches the ``is <>`` default.
   * - _`E1505`
     - An uninstantiated subprogram cannot be called.
   * - _`E1506`
     - A generic subprogram of a component or block cannot be the actual of an entity's generic subprogram in a generic map.
   * - _`W1506`
     - An instance of a generic with a 2008 ``type T`` (or 2019 ``type T is private``) compares values with the actual type's predefined ``=``/``/=`` (LRM 6.5.3), while a different user ``"="`` for the actual type is visible at the instantiation; some simulators use that one.
   * - _`E1507`
     - A generic or port declared with an anonymous type (``type is ...``, VHDL-2019) takes the type of its actual (LRM 6.5.2, 6.5.3), so its association has no conversion function or type conversion on the formal side: the converted type would have to come from the actual, which the conversion hides.
   * - _`E1601`
     - The class of the external name does not allow this use.
   * - _`E1602`
     - The path of an external name is malformed: a package path is ``@lib.pkg.obj``, an absolute path names the top entity first, only generate labels take a globally static index, and a relative path needs an enclosing concurrent region (LRM 8.7).
   * - _`E1610`
     - An object alias must name a static name.
   * - _`E1611`
     - An alias of an overloaded name needs a signature.
   * - _`E1612`
     - The alias declaration does not fit what it names: an object alias needs an identifier designator, a character designator names an enumeration literal, an operator designator names a function with a fitting number of parameters, labels cannot be aliased, and an alias of an external name has no subtype indication (LRM 6.6).
   * - _`E1620`
     - Implicit signal attributes (``'delayed``, ``'stable``, ``'quiet``, ``'transaction``) are not allowed in subprograms.
   * - _`E1621`
     - No predefined or user-defined attribute with this name applies to the prefix.
   * - _`E1622`
     - The entity class of an attribute specification does not match the named entity.
   * - _`E1623`
     - The dimension argument of an array attribute is not a dimension of the prefix.
   * - _`E1625`
     - The type of an attribute may not be an access, file or protected type, or a composite type with such an element (LRM 6.7).
   * - _`E1626`
     - An attribute is given to the same named entity twice, or after an ``others`` or ``all`` specification for its entity class (LRM 7.2).
   * - _`E1630`
     - No declaration matches the signature.
   * - _`E1640`
     - A group template's entity class entry list allows ``<>`` only on its last entry, and a group declaration names a group template (LRM 6.9, 6.10).
   * - _`E1641`
     - A group constituent is not a name of a named entity, its entity class differs from the template's entry, or the list has too many or too few constituents (LRM 6.10).
   * - _`E1650`
     - A disconnection specification names guarded signals declared in the same declarative part, of the type its type mark denotes, and gives a static time expression of type TIME (LRM 7.4).
   * - _`E1651`
     - A guarded signal has at most one disconnection specification (LRM 7.4).
   * - _`E1700`
     - The instantiated name is not a component, an entity or a configuration.
   * - _`E1701`
     - The label is not an instance of the named component.
   * - _`E1702`
     - An instance already bound by a configuration specification cannot be bound again.
   * - _`E1703`
     - A port or generic of the component has no match in the bound entity.
   * - _`W1703`
     - Default binding found an entity in the library of the component declaration (or a directly visible one) and a different one in ``work``; the former is used.
   * - _`E1704`
     - The binding names no entity where one is needed: a configuration specification without an entity aspect, maps without a primary binding, or a block configuration of an instance with no entity.
   * - _`E1801`
     - ``wait`` is not allowed in a process with a sensitivity list or in a function, nor in a procedure that such a process calls, directly or through other procedures (10.2, 11.3); decided at analysis for procedures whose body is in the same design unit; otherwise a function's call is rejected at elaboration and a process's when the ``wait`` executes.
   * - _`E1802`
     - ``return`` is only allowed in a subprogram, with a value exactly in functions.
   * - _`E1803`
     - A name in a sensitivity list or ``wait on`` clause must be a static signal name: every index and slice in it must be static.
   * - _`E1810`
     - A subprogram declared outside a process drives a signal that is not one of its parameters.
   * - _`E1811`
     - ``next`` and ``exit`` must be inside a loop, and a label must name an enclosing loop.
   * - _`E1812`
     - A signal (or sub-element) whose subtype is not resolved has more than one source: two processes, concurrent assignments or ``out`` port associations drive it.
   * - _`E1813`
     - Statements in an entity statement part must be passive: they may not assign signals or call procedures that do.
   * - _`E1820`
     - Case choices must cover every value exactly once.
   * - _`E1830`
     - A pure function reads or writes something outside itself, or calls an impure function.
   * - _`W1830`
     - ``--relaxed`` (R2): a pure function reads or calls something impure.
   * - _`E1901`
     - A mode view must give a mode to every record element exactly once.
   * - _`E1902`
     - A mode view applies to a record subtype.
   * - _`E1903`
     - The port is used against the mode its view gives the element.
   * - _`E1904`
     - A shared variable must be of a protected type (LRM 6.4.2.4); ``--relaxed`` turns this into warning W1904.
   * - _`W1904`
     - ``--relaxed`` (R1): a shared variable of a non-protected type is accepted; the processes using it run serially.
   * - _`E1905`
     - A protected type cannot be the type of a signal, port or file, and has no ``=``.
   * - _`E1906`
     - This declaration is not allowed in a protected type declaration or body (LRM 5.6): the declaration holds method declarations, use clauses, attribute specifications and, in VHDL-2019, private variables and aliases of methods; the body holds no signals or shared variables.
   * - _`E1907`
     - A variable port of a composite (array or record) of protected types is not supported by yavs, and most reference simulators reject it too.
   * - _`E1910`
     - The construct needs VHDL-2019.
   * - _`E1999`
     - The design uses a construct that yavs's semantic analysis does not implement yet.

Elaboration (E2 and W2)
-----------------------

.. list-table::
   :header-rows: 1
   :widths: 12 88

   * - Code
     - Meaning
   * - _`E2001`
     - The lowering to IR does not handle this construct yet; the message names it.
   * - _`E2101`
     - The top level named on the command line does not exist in the work library or cannot be elaborated.
   * - _`E2102`
     - An entity has no architecture to elaborate.
   * - _`E2103`
     - A generic has no value: no actual, no ``-g`` override and no default.
   * - _`E2104`
     - The text of a ``-gNAME=VALUE`` override is not a value of the generic's type.
   * - _`W2105`
     - A ``-gNAME=VALUE`` override names a generic the top level does not have.
   * - _`E2106`
     - An expression that must be evaluated during elaboration cannot be.
   * - _`E2107`
     - Evaluation during elaboration failed: overflow, division by zero or a value out of range.
   * - _`W2108`
     - A component instance is not bound to an entity (LRM 7.3.3) and stays open.
   * - _`E2109`
     - Instances are nested deeper than the limit; the design instantiates itself recursively.
   * - _`E2110`
     - A port actual does not match its formal in shape or length.
   * - _`E2111`
     - A port of an unconstrained subtype has nothing to take its bounds from.
   * - _`E2112`
     - An unresolved signal has several sources (LRM 14.7.2).
   * - _`E2113`
     - A static name selects elements outside its prefix.
   * - _`E2114`
     - A subprogram is called before its body is elaborated (LRM 14.4.2).
   * - _`E2115`
     - A generic value is outside the generic's subtype.
   * - _`E2116`
     - A package needs a body (subprograms, deferred constants) and has none (LRM 4.8).
   * - _`E2117`
     - An external name names no elaborated object (LRM 8.7, 14.4.1).
   * - _`W2118`
     - A constant whose initial value calls an impure function cannot be evaluated during elaboration yet; it is evaluated when the simulation starts, so its calls run after the elaboration-time calls instead of in declaration order.
   * - _`E2119`
     - A top-level entity has a variable port (VHDL-2019): a variable port denotes its actual's protected object, and the top level has no actual (LRM 6.5.6.3).
   * - _`E2120`
     - A TIME physical literal or unit name has a unit below the resolution limit selected with ``yavs run --resolution`` and appears in the design hierarchy, in a package it uses or in code that never runs (LRM 5.2.4.2).
   * - _`E2121`
     - An external name's subtype indication does not match the object it names: the type differs, or a constrained array dimension has another length (LRM 8.7).
   * - _`E2199`
     - A feature needed during elaboration is not implemented yet.

Run time (E3 and W3)
--------------------

.. list-table::
   :header-rows: 1
   :widths: 12 88

   * - Code
     - Meaning
   * - _`E3001`
     - A run-for time, ``now + delay``, is beyond ``TIME'HIGH``.
   * - _`E3002`
     - More delta cycles than the limit ran at one simulated time, usually a zero-delay feedback loop.
   * - _`E3003`
     - A postponed process scheduled a delta cycle (LRM 14.7.5.3).
   * - _`E3004`
     - A malformed signal assignment reached the kernel.
   * - _`E3005`
     - A wait deadline lies before the current time.
   * - _`E3006`
     - An id or element range that does not exist in the model was used.
   * - _`E3007`
     - A resolution function failed.
   * - _`E3008`
     - A port conversion function failed or returned a value of the wrong size.
   * - _`E3009`
     - A VHDL runtime error: bound check, overflow, null access, division by zero and the like.
   * - _`E3010`
     - The model, the simulation configuration or a control request is invalid.
   * - _`W3101`
     - A report of a simulation that did not finish, printed with ``--hang-report`` when the run ends at ``--stop-time`` without ``stop`` or ``finish`` or runs out of events with a process still waiting, and on SIGUSR1 (Unix) for a running simulation.
   * - _`W3102`
     - A process has no wait statement, no sensitivity list and calls no procedure that waits.

Internal errors (E9)
--------------------

.. list-table::
   :header-rows: 1
   :widths: 12 88

   * - Code
     - Meaning
   * - _`E9999`
     - An internal compiler error ("this is a bug in yavs").

See also
--------

* :doc:`troubleshooting`: how to read a diagnostic, and the common problems.
* :doc:`cli-reference`: ``--message-format``, ``-Werror`` and exit statuses.

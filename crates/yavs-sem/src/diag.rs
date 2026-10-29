//! Diagnostic codes of `yavs-sem` (sem.md 8.1) and their `--explain` texts.
//!
//! Every diagnostic sem reports uses one of these constants. A new code gets a
//! constant here **and** an entry in [`EXPLAIN`]; the test at the bottom
//! checks that both lists agree. Codes are never reused for another meaning.

macro_rules! codes {
    ($($(#[$m:meta])* $name:ident = $code:literal, $explain:literal;)*) => {
        $($(#[$m])* pub const $name: &str = $code;)*
        /// `(code, explanation)` for `yavs --explain`.
        pub const EXPLAIN: &[(&str, &str)] = &[$(($code, $explain)),*];
    };
}

codes! {
    // E10xx libraries, units, context clauses
    UNKNOWN_LIBRARY = "E1001", "A library clause or selected name names a library that is not mapped. Map it with `NAME:PATH` or fix the name.";
    UNIT_NOT_FOUND = "E1002", "A design unit was not found in the library it was looked up in. Analyze the file that defines it first, or fix the name.";
    DEPENDENCY_CYCLE = "E1003", "Design units depend on each other in a cycle, so none of them can be analyzed first.";
    NO_PRIMARY = "E1004", "A secondary unit (architecture or package body) names a primary unit that does not exist.";
    WORK_IN_CONTEXT = "E1005", "A context declaration may not name the library `work` (LRM 13.4).";
    NOT_IN_REVISION = "E1007", "A declaration or construct is not available under the revision the file is analyzed with, or means something the design revision does not allow.";
    WRONG_UNIT_KIND = "E1008", "A name denotes a design unit of the wrong kind here, such as a package where an entity is needed.";
    SOURCE_UNREADABLE = "E1009", "A source file named by a library manifest or the std/ieee tree cannot be read.";
    // E11xx scopes and visibility
    NOT_DECLARED = "E1100", "The name is not declared, or not visible, at this point.";
    REDECLARED = "E1101", "A declaration with the same designator (and, for subprograms, the same parameter and result type profile) already exists in this declarative region.";
    USE_CONFLICT = "E1102", "Two use clauses make different declarations with this name potentially visible, so neither is visible (LRM 12.4).";
    HIDDEN = "E1103", "The named entity exists but is hidden at this point.";
    NOT_A_PACKAGE = "E1104", "The prefix of a use clause, context reference or expanded name must denote a library, a package, a package instance or a context declaration (LRM 12.4, 13.4).";
    PRIVATE = "E1105", "The declaration is not in the visible part of its package or protected type.";
    END_LABEL = "E1106", "The name after `end` does not repeat the name of the construct.";
    // E12xx types and subtypes
    NOT_A_TYPE = "E1201", "A type mark must denote a type or subtype.";
    CONSTRAINED_TWICE = "E1210", "An index or element constraint is applied to a dimension that is already constrained.";
    BAD_CONSTRAINT = "E1211", "This kind of constraint does not apply to the type (a range constraint on a composite, an index constraint on a scalar, ...).";
    NOT_FULLY_CONSTRAINED = "E1212", "Signals and variables need a fully constrained subtype.";
    BAD_RECORD_CONSTRAINT = "E1213", "A record constraint names an element that does not exist, or names one twice.";
    INDEX_ARITY = "E1214", "An index constraint has a different number of ranges than the array has dimensions.";
    CONSTRAINT_OUTSIDE = "E1215", "A range or index constraint with static bounds must lie inside the range of its type mark or index subtype (LRM 5.2.1, 5.3.2.2, 6.3); a null range is always compatible.";
    INCOMPLETE = "E1220", "An incomplete type declaration is not completed in the same declarative region, or the incomplete type is used before its full declaration other than as the designated subtype of an access type (LRM 5.4.2).";
    BAD_DESIGNATED = "E1221", "In VHDL-2008 the designated type of an access type may not be a file type or a protected type (LRM 5.4.1); VHDL-2019 allows both.";
    BAD_FILE_ELEMENT = "E1222", "The element type of a file type may not be an access, protected or file type, or contain one.";
    BAD_OBJECT_TYPE = "E1223", "A constant, signal, generic, port or constant or signal parameter may not be of an access or file type, or contain one (LRM 6.4.2.2, 6.4.2.3, 6.5.2): only variables hold access values (also in VHDL-2019; only library `std` declares such parameters). In VHDL-2008 the results and parameters of protected type methods may not be of such types either (LRM 5.6.2).";
    DEFERRED_OUTSIDE_PACKAGE = "E1224", "A constant without a value is a deferred constant, which only a package declaration may hold (LRM 6.4.2.2); anywhere else the constant needs an initial value.";
    BAD_RESOLUTION = "E1225", "A resolution function must be a pure function with one constant parameter of a one-dimensional unconstrained array of the resolved type, returning that type.";
    DEFERRED_EARLY = "E1226", "A deferred constant is named before the end of its full declaration (LRM 6.4.2.2): in an initial value of its package, or anywhere in its package body (subprogram bodies included) before the full declaration. Its value is not known yet there. Only the default expression of a local generic, port or formal parameter may name it.";
    DEFERRED_SUBTYPE = "E1227", "The subtype indication of the full declaration of a deferred constant does not conform to the deferred declaration's (LRM 6.4.2.2): use the same type mark and constraint.";
    NOT_CONFORMING = "E1240", "A subprogram body's specification does not conform to the declaration it completes (LRM 4.10): both must be the same sequence of lexical elements, apart from a numeric literal written with another value spelling and a simple name written as an expanded name. Grouping parameters differently (`x, y : t` against `x : t; y : t`) or writing a mode or class in one of them only is not conforming.";
    MISSING_BODY = "E1230", "A subprogram or protected type is declared but its body is missing, or a deferred constant of a package has no full declaration in the package body (LRM 4.8).";
    BAD_TYPE_DECL = "E1231", "The type definition is not legal, for example a range whose bounds are not of an integer or floating point type.";
    NON_OBJECT_PARAM = "E1232", "A subprogram's parameter list declares only interface constants, signals, variables and files (LRM 4.2.2.1). Type, subprogram and package interfaces belong in its generic list: `function f generic (type t) parameter (x : t) return t`.";
    NAMED_RETURN_TYPE = "E1233", "The type mark after the return identifier of a VHDL-2019 named return value (`return r of t`) denotes a scalar, array or record type: `r` takes the range or bounds of the call's target, which an access or file type does not have (a generic type is checked in each instance).";
    NON_SIGNAL_PORT = "E1234", "A port clause declares only interface signals (LRM 6.5.6.3). Types, subprograms and packages are generics: put them in a `generic (...)` list.";
    GENERIC_HEADER_FORMAL = "E1235", "A formal generic subprogram has no generic list or generic map of its own (LRM 6.5.4): `generic (function f (x : t) return t)` takes its profile from its parameters and return type alone.";
    PARAM_MODE = "E1236", "A subprogram parameter has mode `in`, `out` or `inout` (LRM 4.2.2.1); `buffer` and `linkage` are port modes. A mode view on a signal parameter may still give record elements those modes.";
    SEQUENTIAL_SIGNAL = "E1237", "A process or subprogram declares no signals and no shared variables (LRM 11.3, 4.3), also not in a package declared there or through a package instance there (LRM 4.7, 4.9): such objects belong to the concurrent region around it.";
    NON_SHARED_VARIABLE = "E1238", "A variable declared in an entity, architecture, block, generate or package is a shared variable and needs `shared` (LRM 6.4.2.4); a plain `variable` belongs in a process, subprogram or protected type body.";
    NOT_DECL_ITEM = "E1239", "A component declaration, configuration specification or disconnection specification is not a declarative item of a process or subprogram (LRM 4.3, 11.3): it belongs in an architecture, block, generate or package.";
    // E13xx expressions and overloading
    TYPE_MISMATCH = "E1300", "The expression has a different type than the context requires.";
    NO_OVERLOAD = "E1301", "None of the visible subprograms or literals with this name accepts these operands.";
    AMBIGUOUS = "E1302", "More than one interpretation fits; qualify an operand or use a selected name.";
    BAD_OPERATOR_DECL = "E1303", "An operator symbol that is not a VHDL operator, or an operator function with the wrong number of parameters (LRM 4.5.2).";
    NAMED_RETURN_CONTEXT = "E1304", "A function with a named return value (`return r of t`, VHDL-2019) takes its result subtype from the context of the call: the call must be the value of an assignment, initial value or `return`, an actual of a constrained formal or a qualified expression, of a fully constrained subtype.";
    FORMAL_TWICE = "E1310", "A formal is associated more than once.";
    MISSING_ACTUAL = "E1311", "A formal without a default has no actual.";
    FORMAL_CONV_UNCONSTRAINED = "E1312", "A conversion in the association of a formal that is not fully constrained must give the formal its bounds (5.3.2.2): in the formal part the parameter subtype of the function or the type mark of the type conversion, in the actual part the result subtype of the function or the type mark, must be constrained. A conversion that defines no bounds is an error; one that defines them is accepted.";
    CONV_MISUSE = "E1313", "A conversion in an association that the formal does not allow (6.5.7.1, 4.2.2.3): a formal conversion only for `out` and `inout` variable parameters (and ports other than `in`), an actual conversion only where data flows in, an `inout` parameter converts both ways or not at all, and a signal parameter takes no conversion.";
    AGG_NO_TYPE = "E1320", "The type of an aggregate comes only from its context, and this context does not give one, or gives a type that is not an array or record type.";
    AGG_MISSING = "E1321", "A record aggregate does not give a value for every element.";
    AGG_DUPLICATE = "E1322", "A record aggregate gives a value for an element twice, or has more positional elements than the record has elements.";
    AGG_ELEMENT = "E1323", "An array aggregate element is neither of the element type nor of the aggregate type, an element of a multidimensional aggregate is not an aggregate (or string literal) for the next dimension, or one value is given for record elements of different types.";
    // E1324 is retired (mixed named and positional associations are E1326); never reuse it.
    AGG_OTHERS = "E1325", "`others` must be last and alone, and needs a constrained context subtype.";
    AGG_NONSTATIC_CHOICE = "E1326", "A choice that is not locally static, or a null range choice, must be the only choice of the only association, and named and positional associations do not mix (only a final `others` may follow positional ones).";
    CHOICE_OVERLAP = "E1327", "Choices overlap or repeat a value.";
    CHOICE_UNCOVERED = "E1328", "Choices do not cover every value of the subtype and there is no `others`.";
    AGG_RECORD_CHOICE = "E1329", "A record aggregate choice must be an element name, not an expression or a range.";
    CHAR_NOT_IN_TYPE = "E1330", "A character of a string literal is not a literal of the element type.";
    BITSTRING_DIGITS = "E1331", "Adjusting the length of a bit string literal would drop significant digits (non-zero for unsigned, differing from the sign for signed), or an empty signed bit string cannot be extended.";
    BAD_LITERAL = "E1332", "A literal is malformed in a way the lexer accepts: a digit outside its base, a base outside 2 to 16, a negative exponent in an integer literal, a non-digit in a decimal bit string, or a physical literal with a unit the type does not have.";
    // E1333 is retired (range choices in aggregate targets are accepted, #244); never reuse it.
    AGG_PIECE_DIRECTION = "E1334", "The range choices of array values in one aggregate (`(0 to 1 => a, 3 downto 2 => b)`) have different directions: without a constrained context the aggregate takes the direction of these choices.";
    AGG_TARGET_TWICE = "E1335", "An aggregate target identifies one variable or signal (or a part of it) more than once: by one association with several choices, or by two elements naming the same object (LRM 10.5.2.1, 10.6.2.1).";
    NOT_BOOLEAN = "E1340", "A condition must be BOOLEAN, or of a type with a visible `??` operator.";
    AMBIGUOUS_CONDITION = "E1341", "More than one `??` operator applies to this condition.";
    BAD_CONVERSION = "E1350", "A type conversion is only legal between closely related types.";
    CLASS_MISUSE = "E1360", "The object's class or mode does not allow this use (assigning an `in` port, reading a `linkage` port, reading an `out` port or parameter in a VHDL-1993 file, a variable assignment to a signal, ...).";
    NOT_AN_OBJECT = "E1361", "The name does not denote an object or value here.";
    FORCE_RESOLVED_MEMBER = "E1362", "The target of a force or release assignment is a member (an element, slice or field) of a signal whose composite subtype is resolved as a whole (LRM 10.5.2.1); force or release the whole composite signal.";
    // E14xx static evaluation
    OVERFLOW = "E1401", "Static integer or physical arithmetic overflows 64 bits.";
    LITERAL_RANGE = "E1402", "A literal is too large to represent.";
    OUT_OF_SUBTYPE = "E1403", "A static value lies outside the static subtype it is converted to.";
    LENGTH_MISMATCH = "E1405", "The static lengths of the two sides differ.";
    NOT_STATIC = "E1410", "The expression must be locally static here.";
    // E15xx generics
    NOT_GENERIC = "E1501", "The named unit is not a generic unit.";
    MISSING_GENERIC = "E1502", "A generic without a default has no actual.";
    GENERIC_CLASS = "E1503", "The actual does not belong to the class of the formal generic type.";
    GENERIC_DEFAULT = "E1504", "No unique subprogram matches the `is <>` default.";
    UNINSTANTIATED_CALL = "E1505", "An uninstantiated subprogram cannot be called.";
    GENERIC_SUBPROGRAM_ACTUAL = "E1506", "A generic subprogram of a component or block cannot be the actual of an entity's generic subprogram in a generic map. Name a subprogram declared elsewhere, or rely on the default binding, which passes a component's generic subprogram by name.";
    ANONYMOUS_FORMAL_CONVERSION = "E1507", "A generic or port declared with an anonymous type (`type is ...`, VHDL-2019) takes the type of its actual (LRM 6.5.2, 6.5.3), so its association has no conversion function or type conversion on the formal side: the converted type would have to come from the actual, which the conversion hides. Associate the formal directly and convert in the design.";
    // E16xx names, aliases, attributes, signatures
    EXTERNAL_CLASS = "E1601", "The class of the external name does not allow this use.";
    EXTERNAL_PATH = "E1602", "The path of an external name is malformed: a package path is `@lib.pkg.obj`, an absolute path names the top entity first, only generate labels take a globally static index, and a relative path needs an enclosing concurrent region (LRM 8.7).";
    ALIAS_NOT_STATIC = "E1610", "An object alias must name a static name.";
    ALIAS_SIGNATURE = "E1611", "An alias of an overloaded name needs a signature.";
    BAD_ALIAS = "E1612", "The alias declaration does not fit what it names: an object alias needs an identifier designator, a character designator names an enumeration literal, an operator designator names a function with a fitting number of parameters, labels cannot be aliased, and an alias of an external name has no subtype indication (LRM 6.6).";
    SIGNAL_ATTR_IN_SUBPROGRAM = "E1620", "Implicit signal attributes (`'delayed`, `'stable`, `'quiet`, `'transaction`) are not allowed in subprograms.";
    UNKNOWN_ATTRIBUTE = "E1621", "No predefined or user-defined attribute with this name applies to the prefix.";
    ENTITY_CLASS = "E1622", "The entity class of an attribute specification does not match the named entity.";
    ATTR_DIMENSION = "E1623", "The dimension argument of an array attribute is not a dimension of the prefix.";
    ATTR_TYPE = "E1625", "The type of an attribute may not be an access, file or protected type, or a composite type with such an element (LRM 6.7).";
    ATTR_TWICE = "E1626", "An attribute is given to the same named entity twice, or after an `others` or `all` specification for its entity class (LRM 7.2).";
    GROUP_TEMPLATE = "E1640", "A group template's entity class entry list allows `<>` only on its last entry, and a group declaration names a group template (LRM 6.9, 6.10).";
    GROUP_CONSTITUENT = "E1641", "A group constituent is not a name of a named entity, its entity class differs from the template's entry, or the list has too many or too few constituents (LRM 6.10).";
    DISCONNECT_SPEC = "E1650", "A disconnection specification names guarded signals declared in the same declarative part, of the type its type mark denotes, and gives a static time expression of type TIME (LRM 7.4).";
    DISCONNECT_TWICE = "E1651", "A guarded signal has at most one disconnection specification (LRM 7.4).";
    SIGNATURE_NONE = "E1630", "No declaration matches the signature.";
    // E17xx configurations, binding, components
    NOT_INSTANTIABLE = "E1700", "The instantiated name is not a component, an entity or a configuration.";
    NOT_AN_INSTANCE = "E1701", "The label is not an instance of the named component.";
    REBINDING = "E1702", "An instance already bound by a configuration specification cannot be bound again.";
    BINDING_MISMATCH = "E1703", "A port or generic of the component has no match in the bound entity.";
    NO_ENTITY_BOUND = "E1704", "The binding names no entity where one is needed: a configuration specification without an entity aspect, maps without a primary binding, or a block configuration of an instance with no entity.";
    // E18xx statements, processes, drivers
    BAD_WAIT = "E1801", "`wait` is not allowed in a process with a sensitivity list or in a function, nor in a procedure that such a process calls, directly or through other procedures (10.2, 11.3); decided at analysis for procedures whose body is in the same design unit; otherwise a function's call is rejected at elaboration and a process's when the `wait` executes.";
    BAD_RETURN = "E1802", "`return` is only allowed in a subprogram, with a value exactly in functions.";
    SENSITIVITY_NOT_STATIC = "E1803", "A name in a sensitivity list or `wait on` clause must be a static signal name: every index and slice in it must be static.";
    DRIVER_OUTSIDE_PROCESS = "E1810", "A subprogram declared outside a process drives a signal that is not one of its parameters.";
    MULTIPLE_SOURCES = "E1812", "A signal (or sub-element) whose subtype is not resolved has more than one source: two processes, concurrent assignments or `out` port associations drive it. Drive it from one place or give it a resolved subtype such as `std_logic`.";
    NOT_PASSIVE = "E1813", "Statements in an entity statement part must be passive: they may not assign signals or call procedures that do.";
    NEXT_OUTSIDE_LOOP = "E1811", "`next` and `exit` must be inside a loop, and a label must name an enclosing loop.";
    CASE_COVERAGE = "E1820", "Case choices must cover every value exactly once.";
    PURE_SIDE_EFFECT = "E1830", "A pure function reads or writes something outside itself, or calls an impure function.";
    // E19xx protected types, views, 2019
    VIEW_ELEMENT = "E1901", "A mode view must give a mode to every record element exactly once.";
    VIEW_NOT_RECORD = "E1902", "A mode view applies to a record subtype.";
    VIEW_MODE = "E1903", "The port is used against the mode its view gives the element.";
    SHARED_NOT_PROTECTED = "E1904", "A shared variable must be of a protected type (LRM 6.4.2.4); `--relaxed` turns this into warning W1904.";
    PROTECTED_MISUSE = "E1905", "A protected type cannot be the type of a signal, port or file, and has no `=`. A 2019 record with a protected element has only protected elements (LRM 5.3.1).";
    PROTECTED_ITEM = "E1906", "This declaration is not allowed in a protected type declaration or body (LRM 5.6): the declaration holds method declarations, use clauses, attribute specifications and, in VHDL-2019, private variables and aliases of methods; the body holds no signals or shared variables.";
    PROTECTED_COMPOSITE_PORT = "E1907", "A variable port of a composite (array or record) of protected types is not supported by yavs, and most reference simulators reject it too. Use a variable port of the protected type, one per object.";
    NEEDS_2019 = "E1910", "The construct needs VHDL-2019.";
    NOT_IMPLEMENTED = "E1999", "The design uses a construct that yavs's semantic analysis does not implement yet. This is a limitation of yavs, not an error in the design; the note names the slice of the semantic analysis plan that is to implement it. Please report it with the construct if no issue exists yet.";
    // Warnings
    W_CHANGED_MEANING = "W1008", "The construct means something different under the design's VHDL-2019 std/ieee than under VHDL-2008, for example `integer'high`.";
    W_OVERFLOW = "W1401", "Static arithmetic overflows in a context that does not require the value; the runtime check stays.";
    W_STATIC_OUTSIDE = "W1403", "A static operand of a qualified expression or type conversion lies outside the subtype it is converted to, or an array with static bounds converted to an unconstrained array type has bounds outside the target's index subtype; the code may be dead, so this is checked again at runtime.";
    W_STATIC_INDEX = "W1404", "A static index is outside the static range of the array; the code may be dead, so this is checked again at runtime.";
    W_UNUSED_GENERIC = "W1501", "A generic is never used.";
    W_GENERIC_EQ = "W1506", "An instance of a generic with a 2008 `type T` (or 2019 `type T is private`) compares values with the actual type's predefined `=`/`/=` (LRM 6.5.3), while a different user `\"=\"` for the actual type is visible at the instantiation; sim1 would use that one (decision U-6). To use the user operator, declare a formal `function \"=\" (l, r : t) return boolean is <>`.";
    W_DEFAULT_BINDING = "W1703", "Default binding found an entity in the library of the component declaration (or a directly visible one) and a different one in `work`; the former is used.";
    W_OTHERS_TARGET = "W1325", "`--relaxed` (R5): an aggregate with `others` is assigned to an object whose subtype is unconstrained (a port, a formal); it takes the bounds of the object at run time.";
    W_SIGNAL_ACTUAL_NOT_STATIC = "W1361", "`--relaxed` (R6): the actual of a signal parameter is not a static name (an index by a variable or loop parameter); LRM 4.2.2.3 requires one, the call passes the element the index selects at the call.";
    W_PURE_IMPURE = "W1830", "`--relaxed` (R2): a pure function reads or calls something impure.";
    W_SHARED_NOT_PROTECTED = "W1904", "`--relaxed` (R1): a shared variable of a non-protected type is accepted; the processes using it run serially.";
}

/// Internal compiler error (shared with every stage).
pub const INTERNAL: &str = "E9999";

/// The `--explain` text for a code.
pub fn explain(code: &str) -> Option<&'static str> {
    EXPLAIN.iter().find(|(c, _)| *c == code).map(|(_, e)| *e)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn codes_are_unique_and_in_range() {
        let mut seen = std::collections::HashSet::new();
        for (c, e) in EXPLAIN {
            assert!(seen.insert(*c), "duplicate code {c}");
            assert!(
                c.starts_with("E1") || c.starts_with("W1"),
                "{c} outside E1xxx/W1xxx"
            );
            assert!(!e.is_empty());
        }
    }
}

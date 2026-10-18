//! Diagnostic codes of `rsim-sem` (sem.md 8.1) and their `--explain` texts.
//!
//! Every diagnostic sem reports uses one of these constants. A new code gets a
//! constant here **and** an entry in [`EXPLAIN`]; the test at the bottom
//! checks that both lists agree. Codes are never reused for another meaning.

macro_rules! codes {
    ($($(#[$m:meta])* $name:ident = $code:literal, $explain:literal;)*) => {
        $($(#[$m])* pub const $name: &str = $code;)*
        /// `(code, explanation)` for `rsim --explain`.
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
    INCOMPLETE = "E1220", "An incomplete type declaration is not completed in the same declarative region.";
    BAD_FILE_ELEMENT = "E1222", "The element type of a file type may not be an access, protected or file type, or contain one.";
    BAD_RESOLUTION = "E1225", "A resolution function must be a pure function with one constant parameter of a one-dimensional unconstrained array of the resolved type, returning that type.";
    MISSING_BODY = "E1230", "A subprogram or protected type is declared but its body is missing.";
    BAD_TYPE_DECL = "E1231", "The type definition is not legal, for example a range whose bounds are not of an integer or floating point type.";
    // E13xx expressions and overloading
    TYPE_MISMATCH = "E1300", "The expression has a different type than the context requires.";
    NO_OVERLOAD = "E1301", "None of the visible subprograms or literals with this name accepts these operands.";
    AMBIGUOUS = "E1302", "More than one interpretation fits; qualify an operand or use a selected name.";
    BAD_OPERATOR_DECL = "E1303", "An operator symbol that is not a VHDL operator, or an operator function with the wrong number of parameters (LRM 4.5.2).";
    FORMAL_TWICE = "E1310", "A formal is associated more than once.";
    MISSING_ACTUAL = "E1311", "A formal without a default has no actual.";
    AGG_NO_TYPE = "E1320", "The type of an aggregate comes only from its context, and this context does not give one.";
    AGG_MISSING = "E1321", "A record aggregate does not give a value for every element.";
    AGG_DUPLICATE = "E1322", "A record aggregate gives a value for an element twice.";
    AGG_ELEMENT = "E1323", "An array aggregate element is neither of the element type nor of the aggregate type.";
    AGG_MIXED = "E1324", "Named and positional associations are mixed in an array aggregate.";
    AGG_OTHERS = "E1325", "`others` must be last and alone, and needs a constrained context subtype.";
    AGG_NONSTATIC_CHOICE = "E1326", "A choice that is not locally static must be the only choice of the only association.";
    CHOICE_OVERLAP = "E1327", "Choices overlap or repeat a value.";
    CHOICE_UNCOVERED = "E1328", "Choices do not cover every value of the subtype and there is no `others`.";
    CHAR_NOT_IN_TYPE = "E1330", "A character of a string literal is not a literal of the element type.";
    BITSTRING_DIGITS = "E1331", "Adjusting the length of a bit string literal would drop significant digits (non-zero for unsigned, differing from the sign for signed), or an empty signed bit string cannot be extended.";
    BAD_LITERAL = "E1332", "A literal is malformed in a way the lexer accepts: a digit outside its base, a base outside 2 to 16, a negative exponent in an integer literal, a non-digit in a decimal bit string, or a physical literal with a unit the type does not have.";
    NOT_BOOLEAN = "E1340", "A condition must be BOOLEAN, or of a type with a visible `??` operator.";
    AMBIGUOUS_CONDITION = "E1341", "More than one `??` operator applies to this condition.";
    BAD_CONVERSION = "E1350", "A type conversion is only legal between closely related types.";
    CLASS_MISUSE = "E1360", "The object's class or mode does not allow this use (assigning an `in` port, reading a `linkage` port, a variable assignment to a signal, ...).";
    NOT_AN_OBJECT = "E1361", "The name does not denote an object or value here.";
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
    SIGNATURE_NONE = "E1630", "No declaration matches the signature.";
    // E17xx configurations, binding, components
    NOT_INSTANTIABLE = "E1700", "The instantiated name is not a component, an entity or a configuration.";
    NOT_AN_INSTANCE = "E1701", "The label is not an instance of the named component.";
    REBINDING = "E1702", "An instance already bound by a configuration specification cannot be bound again.";
    BINDING_MISMATCH = "E1703", "A port or generic of the component has no match in the bound entity.";
    // E18xx statements, processes, drivers
    BAD_WAIT = "E1801", "`wait` is not allowed in a process with a sensitivity list or in a function.";
    BAD_RETURN = "E1802", "`return` is only allowed in a subprogram, with a value exactly in functions.";
    DRIVER_OUTSIDE_PROCESS = "E1810", "A subprogram declared outside a process drives a signal that is not one of its parameters.";
    NEXT_OUTSIDE_LOOP = "E1811", "`next` and `exit` must be inside a loop, and a label must name an enclosing loop.";
    CASE_COVERAGE = "E1820", "Case choices must cover every value exactly once.";
    PURE_SIDE_EFFECT = "E1830", "A pure function reads or writes something outside itself, or calls an impure function.";
    // E19xx protected types, views, 2019
    VIEW_ELEMENT = "E1901", "A mode view must give a mode to every record element exactly once.";
    VIEW_NOT_RECORD = "E1902", "A mode view applies to a record subtype.";
    VIEW_MODE = "E1903", "The port is used against the mode its view gives the element.";
    SHARED_NOT_PROTECTED = "E1904", "A shared variable must be of a protected type (LRM 6.4.2.4); `--relaxed` turns this into warning W1904.";
    PROTECTED_MISUSE = "E1905", "A protected type cannot be the type of a signal, port or file, and has no `=`.";
    PROTECTED_ITEM = "E1906", "This declaration is not allowed in a protected type declaration or body (LRM 5.6): the declaration holds method declarations, use clauses, attribute specifications and, in VHDL-2019, private variables and aliases of methods; the body holds no signals or shared variables.";
    NEEDS_2019 = "E1910", "The construct needs VHDL-2019.";
    // Warnings
    W_CHANGED_MEANING = "W1008", "The construct means something different under the design's VHDL-2019 std/ieee than under VHDL-2008, for example `integer'high`.";
    W_OVERFLOW = "W1401", "Static arithmetic overflows in a context that does not require the value; the runtime check stays.";
    W_STATIC_INDEX = "W1404", "A static index is outside the static range of the array; the code may be dead, so this is checked again at runtime.";
    W_UNUSED_GENERIC = "W1501", "A generic is never used.";
    W_DEFAULT_BINDING = "W1703", "Default binding found a directly visible entity and a different one in `work`; the directly visible one is used.";
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

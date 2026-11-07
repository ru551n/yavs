//! Diagnostic constructors and the code table.

use crate::{CodeInfo, Diag, Severity, Span};

impl Diag {
    pub(crate) fn new(
        code: &'static str,
        severity: Severity,
        message: impl Into<String>,
        span: Option<Span>,
    ) -> Diag {
        Diag {
            code,
            severity,
            message: message.into(),
            span,
            label: None,
            notes: Vec::new(),
            help: None,
        }
    }
    pub(crate) fn error(code: &'static str, message: impl Into<String>, span: Span) -> Diag {
        Diag::new(code, Severity::Error, message, Some(span))
    }
    pub(crate) fn warning(code: &'static str, message: impl Into<String>, span: Span) -> Diag {
        Diag::new(code, Severity::Warning, message, Some(span))
    }
    pub(crate) fn label(mut self, l: impl Into<String>) -> Diag {
        self.label = Some(l.into());
        self
    }
    pub(crate) fn note(mut self, n: impl Into<String>) -> Diag {
        self.notes.push(n.into());
        self
    }
    pub(crate) fn help(mut self, h: impl Into<String>) -> Diag {
        self.help = Some(h.into());
        self
    }
}

macro_rules! code {
    ($c:literal, $t:literal, $e:literal) => {
        CodeInfo {
            code: $c,
            title: $t,
            explain: $e,
        }
    };
}

pub(crate) const TABLE: &[CodeInfo] = &[
    code!(
        "E4001",
        "instantiated module is encrypted",
        "Reserved for elaboration: a design instantiates a module whose body is a protected envelope (IEEE 1735). yavs never decrypts protected IP, so the module cannot be simulated. \
         Use the vendor's unencrypted simulation model, or simulate this block with a tool that holds the vendor's key. Analysis only records the module and warns (W4007)."
    ),
    code!(
        "E4002",
        "SystemVerilog source file",
        "yavs reads Verilog (IEEE 1364-2005) only. A file with a SystemVerilog extension (.sv, .svh) is rejected. \
         If the file is plain Verilog with an unusual extension, pass --lang=verilog."
    ),
    code!(
        "E4003",
        "SystemVerilog construct in a Verilog file",
        "The file uses a construct that exists only in SystemVerilog (IEEE 1800), named in the message. yavs reads Verilog (IEEE 1364-2005) only."
    ),
    code!(
        "E4004",
        "unbalanced conditional compilation",
        "A `ifdef, `ifndef, `elsif, `else or `endif does not match: a missing `endif at the end of the file, an `else or `endif without an opening `ifdef, \
         or an `elsif/`else after an `else. The conditional must start and end in the same file."
    ),
    code!(
        "E4005",
        "include file not found",
        "The file named by `include was not found in the directory of the including file or in any -I directory. The message lists the directories searched."
    ),
    code!(
        "W4006",
        "no `timescale in force",
        "The module has no `timescale directive before it and none came from an earlier file, so the default (1ns/1ps, or --timescale) applies. \
         IEEE 1364-2005 leaves this to the tool, and delays in the module scale with the default."
    ),
    code!(
        "E4007",
        "undefined macro",
        "A text macro is used with `NAME but no `define NAME is in force at that point. Define it in the source, or with -D NAME[=VALUE]."
    ),
    code!(
        "W4007",
        "encrypted module body recorded",
        "The source contains a protected envelope (`pragma protect begin_protected ... end_protected, or `protect ... `endprotect, or `protected ... `endprotected). \
         The module is recorded as an encrypted unit with the ports of its readable header, and analysis succeeds; an envelope before any module header records nothing. \
         Instantiating the module is an error at elaboration (E4001). yavs never decrypts protected IP."
    ),
    code!(
        "E4008",
        "macro definition or call is malformed",
        "A `define has no name or a bad formal argument list, or a macro call has the wrong number of arguments or an unterminated argument list."
    ),
    code!(
        "E4009",
        "malformed compiler directive",
        "A compiler directive has an operand that is missing or not valid, for example `timescale 3ns/1ps (the magnitude must be 1, 10 or 100) \
         or a `default_nettype that is not a net type."
    ),
    code!(
        "E4010",
        "unsupported compiler directive",
        "The directive is outside IEEE 1364-2005 as yavs supports it (for example `uselib, or `config library maps). It is not skipped silently."
    ),
    code!(
        "E4011",
        "nesting too deep",
        "Macro expansion or `include nesting exceeded its limit; a macro that expands to itself, or files that include each other, are the usual cause."
    ),
    code!(
        "E4012",
        "cannot read source file",
        "The source file could not be opened or read, or its extension is not a Verilog one and --lang=verilog was not given."
    ),
    code!(
        "W4009",
        "protection marker ignored",
        "`pragma protect begin or `pragma protect end marks plaintext that a tool would encrypt. The marker is ignored and the plaintext is compiled."
    ),
    code!(
        "E4014",
        "lexical error",
        "A character, string, comment, number or identifier is malformed: an unterminated string or block comment, an illegal character, or a bad numeric literal."
    ),
    code!(
        "W4015",
        "timing checks are not evaluated",
        "The design defines XIL_TIMING and contains timing checks ($setup, $hold, $setuphold, ...). They are parsed and ignored: functional simulation never evaluates them."
    ),
    code!(
        "E4100",
        "syntax error",
        "The source does not match the grammar of IEEE 1364-2005 at this point. The message says what was expected. After the first error in a module the rest of that module is skipped."
    ),
    code!(
        "E4101",
        "unsupported Verilog construct",
        "The construct is legal 1364-2005 but outside what yavs reads, named in the message (for example config blocks and macromodule)."
    ),
    code!(
        "E4200",
        "duplicate module or primitive",
        "Two modules or primitives with the same name are declared in the files of one analysis."
    ),
    code!(
        "E4201",
        "undeclared identifier",
        "An identifier is used in an expression but declared nowhere in the module, and `default_nettype none is in force, so no implicit net is created (clause 4.5)."
    ),
    code!(
        "E4202",
        "port declaration mismatch",
        "A port of a module header with a non-ANSI port list has no direction declaration in the body, or a body declaration names a port the header does not list, \
         or a port is declared twice (clause 12.3)."
    ),
    code!(
        "E4203",
        "duplicate declaration",
        "A name is declared twice in the same module scope (clause 12.1.2)."
    ),
];

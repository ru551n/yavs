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
        "A design instantiates a Verilog module whose body is encrypted (an IEEE 1735 protected envelope, `` `pragma protect begin_protected `` or the older `` `protect `` forms), so it cannot be simulated. yavs never decrypts protected IP. Analysis records such a module by its name (warning `W4007`), so a library of vendor files analyses; the error comes when something instantiates it, and names the instance. Simulate the vendor's unencrypted model instead, or the block with a tool that holds the vendor's key."
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
        "A `` `ifdef ``, `` `ifndef ``, `` `elsif ``, `` `else `` or `` `endif `` does not match: a missing `` `endif `` at the end of the file, an `` `else `` or `` `endif `` without an opening `` `ifdef ``, \
         or an `` `elsif ``/`` `else `` after an `` `else ``. The conditional must start and end in the same file. The same code covers a `` `begin_keywords `` that nothing closes and an `` `end_keywords `` that closes nothing (clause 19.11)."
    ),
    code!(
        "E4005",
        "include file not found",
        "The file named by `` `include `` was not found in the directory of the including file or in any -I directory. The message lists the directories searched."
    ),
    code!(
        "W4006",
        "no timescale in force",
        "The module has no `` `timescale `` directive before it and none came from an earlier file, so the default (1ns/1ps, or --timescale) applies. \
         IEEE 1364-2005 (19.8) leaves the time unit of such a module to the tool and calls a design in which only some modules have a timescale an error; yavs warns. Delays in the module scale with the default."
    ),
    code!(
        "E4007",
        "undefined macro",
        "A text macro is used with `` `NAME `` but no `` `define NAME `` is in force at that point. Define it in the source, or with -D NAME[=VALUE]."
    ),
    code!(
        "W4007",
        "encrypted module body recorded",
        "A Verilog file holds an encrypted (IEEE 1735 protected) module body, `` `pragma protect begin_protected `` or the older `` `protect `` forms. yavs never decrypts protected IP, so it does not read the encoded block. The module is recorded by its name and ports, so a library of vendor files analyses; a design that instantiates it fails with `E4001`. One warning is given per file. Use `-Wno-W4007` to silence it."
    ),
    code!(
        "E4008",
        "macro definition or call is malformed",
        "A `` `define `` has no name, names a compiler directive (clause 19.3.1) or has a bad formal argument list, or a macro call has the wrong number of arguments or an unterminated argument list."
    ),
    code!(
        "E4009",
        "malformed compiler directive",
        "A compiler directive has an operand that is missing or not valid, for example `` `timescale 3ns/1ps `` (the magnitude must be 1, 10 or 100) \
         a `` `default_nettype `` that is not a net type, a `` `line `` without its number, file name and level, a `` `begin_keywords `` with an unknown version, or text after an `` `include `` on its line."
    ),
    code!(
        "E4010",
        "unsupported compiler directive",
        "The directive is outside IEEE 1364-2005 as yavs supports it (for example `` `uselib ``, or `` `config `` library maps). It is not skipped silently."
    ),
    code!(
        "E4011",
        "nesting too deep",
        "Macro expansion or `` `include `` nesting exceeded its limit; a macro that expands to itself, or files that include each other, are the usual cause."
    ),
    code!(
        "E4012",
        "cannot read source file",
        "The source file could not be opened or read, or its extension is not a Verilog one and --lang=verilog was not given."
    ),
    code!(
        "W4009",
        "protection marker ignored",
        "A `` `pragma protect begin `` / `` `pragma protect end `` pair marks plaintext that a tool was to encrypt. The file is not encrypted: yavs ignores the markers and compiles the plaintext between them. One warning is given per file."
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
        "W4016",
        "compiler directive ignored",
        "A compiler directive of fault simulation or netlist size (`suppress_faults`, `enable_portfaults`, `accelerate`, `expand_vectornets`, `remove_netnames` and their negations) is not part of IEEE 1364-2005 and does not change a functional simulation. yavs ignores it, as the other tools do."
    ),
    code!(
        "W4017",
        "default value of a port",
        "An input or inout port, or an output without a variable type, is given a value in the port list (`input a = 1'b0`). IEEE 1364-2005 (12.3.4) allows a value only for an output variable; the construct is SystemVerilog. yavs accepts it, as the other tools do, and warns once per file."
    ),
    code!(
        "W4205",
        "task enabled with empty parentheses",
        "A task enable such as `t();` has parentheses and no argument. IEEE 1364-2005 (10.2.2) says a task without arguments is enabled without a list; yavs accepts it, as the other tools do, and warns."
    ),
    code!(
        "W4206",
        "declaration in an unnamed block",
        "A `begin` or `fork` block without a name declares variables, parameters or events. In IEEE 1364-2005 (9.8.1, 9.8.2) only a named block has declarations; yavs accepts it, as the other tools do, and warns. Name the block."
    ),
    code!(
        "W4207",
        "variable declaration assignment in a block or task",
        "A variable is given a value where it is declared inside a task, function or block. IEEE 1364-2005 (6.2.1) allows that at module level only; yavs accepts it, as the other tools do, and warns. On an array it is still an error."
    ),
    code!(
        "E4100",
        "syntax error",
        "The source does not match the grammar of IEEE 1364-2005 at this point. The message says what was expected. After the first error in a module the rest of that module is skipped."
    ),
    code!(
        "E4101",
        "unsupported Verilog construct",
        "The construct is legal 1364-2005 but outside what yavs reads, named in the message (for example config blocks)."
    ),
    code!(
        "E4200",
        "duplicate module or primitive",
        "Two modules or primitives with the same name are declared in the files of one analysis."
    ),
    code!(
        "E4201",
        "undeclared identifier",
        "An identifier is used in an expression but declared nowhere in the module, and `` `default_nettype none `` is in force, so no implicit net is created (clause 4.5)."
    ),
    code!(
        "E4202",
        "port declaration mismatch",
        "A port of a module header with a non-ANSI port list has no direction declaration in the body, or a body declaration names a port the header does not list, \
         a port is declared twice or defined twice by name, an input or inout port is declared `reg` (clause 12.3), or a module that declares its ports in the header declares one in the body (clause 12.3.4)."
    ),
    code!(
        "E4203",
        "duplicate declaration",
        "A name is declared twice in the same scope: a module, task, function, named block, generate block or specify block (clause 4.11), or a port of the module header is declared again in the body (clause 12.3.4)."
    ),
    code!(
        "E4204",
        "generate loop variable is not a genvar",
        "The variable of a generate `for` loop is declared with `genvar` (clause 12.4.1). A `reg` or `integer` cannot be the variable of a generate loop."
    ),
];

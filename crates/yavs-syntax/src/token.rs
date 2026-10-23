//! Tokens and reserved words.

use crate::Std;

macro_rules! keywords {
    ($($kw:ident = $s:literal,)*) => {
        /// A reserved word.
        #[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
        #[derive(serde::Serialize, serde::Deserialize)]
        pub enum Kw { $($kw,)* }

        impl Kw {
            pub fn as_str(self) -> &'static str {
                match self { $(Kw::$kw => $s,)* }
            }

            /// Reserved words by first letter: most identifiers are rejected after a few
            /// comparisons instead of one per reserved word.
            fn from_lower(s: &[u8]) -> Option<Kw> {
                static BY_LETTER: std::sync::LazyLock<[Vec<(&'static [u8], Kw)>; 26]> =
                    std::sync::LazyLock::new(|| {
                        let mut t: [Vec<(&'static [u8], Kw)>; 26] = Default::default();
                        $(t[($s.as_bytes()[0] - b'a') as usize].push(($s.as_bytes(), Kw::$kw));)*
                        t
                    });
                let c = *s.first()?;
                if !c.is_ascii_lowercase() {
                    return None;
                }
                BY_LETTER[(c - b'a') as usize]
                    .iter()
                    .find(|(k, _)| *k == s)
                    .map(|&(_, kw)| kw)
            }
        }
    };
}

keywords! {
    Abs = "abs", Access = "access", After = "after", Alias = "alias", All = "all",
    And = "and", Architecture = "architecture", Array = "array", Assert = "assert",
    Assume = "assume", AssumeGuarantee = "assume_guarantee", Attribute = "attribute",
    Begin = "begin", Block = "block", Body = "body", Buffer = "buffer", Bus = "bus",
    Case = "case", Component = "component", Configuration = "configuration",
    Constant = "constant", Context = "context", Cover = "cover", Default = "default",
    Disconnect = "disconnect", Downto = "downto", Else = "else", Elsif = "elsif",
    End = "end", Entity = "entity", Exit = "exit", Fairness = "fairness", File = "file",
    For = "for", Force = "force", Function = "function", Generate = "generate",
    Generic = "generic", Group = "group", Guarded = "guarded", If = "if",
    Impure = "impure", In = "in", Inertial = "inertial", Inout = "inout", Is = "is",
    Label = "label", Library = "library", Linkage = "linkage", Literal = "literal",
    Loop = "loop", Map = "map", Mod = "mod", Nand = "nand", New = "new", Next = "next",
    Nor = "nor", Not = "not", Null = "null", Of = "of", On = "on", Open = "open",
    Or = "or", Others = "others", Out = "out", Package = "package",
    Parameter = "parameter", Port = "port", Postponed = "postponed", Private = "private",
    Procedure = "procedure", Process = "process", Property = "property",
    Protected = "protected", Pure = "pure", Range = "range", Record = "record",
    Register = "register", Reject = "reject", Release = "release", Rem = "rem",
    Report = "report", Restrict = "restrict", RestrictGuarantee = "restrict_guarantee",
    Return = "return", Rol = "rol", Ror = "ror", Select = "select", Sequence = "sequence",
    Severity = "severity", Shared = "shared", Signal = "signal", Sla = "sla", Sll = "sll",
    Sra = "sra", Srl = "srl", Strong = "strong", Subtype = "subtype", Then = "then",
    To = "to", Transport = "transport", Type = "type", Unaffected = "unaffected",
    Units = "units", Until = "until", Use = "use", Variable = "variable", View = "view",
    Vmode = "vmode", Vprop = "vprop", Vunit = "vunit", Wait = "wait", When = "when",
    While = "while", With = "with", Xnor = "xnor", Xor = "xor",
}

impl Kw {
    /// Look up a reserved word of `std`, given the identifier's lower-case text.
    pub fn lookup(lower: &str, std: Std) -> Option<Kw> {
        Kw::lookup_bytes(lower.as_bytes(), std)
    }

    /// [`Kw::lookup`] for the lower-case bytes of an ASCII word.
    pub fn lookup_bytes(lower: &[u8], std: Std) -> Option<Kw> {
        let kw = Kw::from_lower(lower)?;
        match kw {
            Kw::Private | Kw::View if std < Std::Vhdl2019 => None,
            // 15.10: the words 1076-2008 reserved (the PSL words and `context`, `default`,
            // `force`, `parameter`, `protected`, `release`) are plain identifiers in 1993.
            Kw::Assume
            | Kw::AssumeGuarantee
            | Kw::Context
            | Kw::Cover
            | Kw::Default
            | Kw::Fairness
            | Kw::Force
            | Kw::Parameter
            | Kw::Property
            | Kw::Protected
            | Kw::Release
            | Kw::Restrict
            | Kw::RestrictGuarantee
            | Kw::Sequence
            | Kw::Strong
            | Kw::Vmode
            | Kw::Vprop
            | Kw::Vunit
                if std < Std::Vhdl2008 =>
            {
                None
            }
            kw => Some(kw),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Tok {
    /// Basic identifier.
    Ident,
    /// Extended identifier `\like this\`.
    ExtIdent,
    Kw(Kw),
    /// Decimal or based literal.
    Abstract,
    Char,
    Str,
    BitStr,
    /// Backtick followed by an identifier, such as `` `if ``.
    Directive,
    Amp,
    Tick,
    LParen,
    RParen,
    Star,
    Plus,
    Comma,
    Minus,
    Dot,
    Slash,
    Colon,
    Semi,
    Lt,
    Eq,
    Gt,
    Bar,
    LBracket,
    RBracket,
    Question,
    At,
    /// `{` and `}` only occur in PSL.
    LBrace,
    RBrace,
    Caret,
    /// `=>`
    Arrow,
    /// `->`, PSL implication.
    Imply,
    /// `<->`, PSL equivalence.
    Equiv,
    /// `**`
    Pow,
    /// `:=`
    Assign,
    /// `/=`
    Ne,
    /// `>=`
    Ge,
    /// `<=`
    Le,
    /// `<>`
    Box,
    /// `??`
    Cond,
    /// `?=`
    MatchEq,
    /// `?/=`
    MatchNe,
    /// `?<`
    MatchLt,
    /// `?<=`
    MatchLe,
    /// `?>`
    MatchGt,
    /// `?>=`
    MatchGe,
    /// `<<`
    LtLt,
    /// `>>`
    GtGt,
    Eof,
}

impl Tok {
    /// How the token is named in "expected X, found Y" messages.
    pub fn describe(self) -> String {
        let s = match self {
            Tok::Ident | Tok::ExtIdent => "identifier",
            Tok::Kw(k) => return format!("`{}`", k.as_str()),
            Tok::Abstract => "number",
            Tok::Char => "character literal",
            Tok::Str => "string literal",
            Tok::BitStr => "bit string literal",
            Tok::Directive => "tool directive",
            Tok::Eof => "end of file",
            t => return format!("`{}`", t.delimiter()),
        };
        s.to_string()
    }

    fn delimiter(self) -> &'static str {
        match self {
            Tok::Amp => "&",
            Tok::Tick => "'",
            Tok::LParen => "(",
            Tok::RParen => ")",
            Tok::Star => "*",
            Tok::Plus => "+",
            Tok::Comma => ",",
            Tok::Minus => "-",
            Tok::Dot => ".",
            Tok::Slash => "/",
            Tok::Colon => ":",
            Tok::Semi => ";",
            Tok::Lt => "<",
            Tok::Eq => "=",
            Tok::Gt => ">",
            Tok::Bar => "|",
            Tok::LBracket => "[",
            Tok::RBracket => "]",
            Tok::Question => "?",
            Tok::At => "@",
            Tok::LBrace => "{",
            Tok::RBrace => "}",
            Tok::Caret => "^",
            Tok::Arrow => "=>",
            Tok::Imply => "->",
            Tok::Equiv => "<->",
            Tok::Pow => "**",
            Tok::Assign => ":=",
            Tok::Ne => "/=",
            Tok::Ge => ">=",
            Tok::Le => "<=",
            Tok::Box => "<>",
            Tok::Cond => "??",
            Tok::MatchEq => "?=",
            Tok::MatchNe => "?/=",
            Tok::MatchLt => "?<",
            Tok::MatchLe => "?<=",
            Tok::MatchGt => "?>",
            Tok::MatchGe => "?>=",
            Tok::LtLt => "<<",
            Tok::GtGt => ">>",
            _ => "?",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Token {
    pub kind: Tok,
    pub start: u32,
    pub end: u32,
}

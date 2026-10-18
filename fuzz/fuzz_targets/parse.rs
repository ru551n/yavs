//! Lexer, conditional analysis and parser must never panic or hang.
#![no_main]

use libfuzzer_sys::fuzz_target;
use rsim_diag::{MessageFormat, SourceMap};
use rsim_syntax::{Options, Std, decode_source, parse_file};

fuzz_target!(|data: &[u8]| {
    let text = decode_source(data.to_vec());
    let mut sources = SourceMap::default();
    let file = sources.add("fuzz.vhd", text.clone());
    for std in [Std::Vhdl2008, Std::Vhdl2019] {
        let parsed = parse_file(file, &text, &Options { std, defines: Vec::new() });
        // Rendering exercises every span the parser produced.
        for d in &parsed.diags {
            rsim_diag::render(&sources, d, MessageFormat::Human, false);
        }
    }
});

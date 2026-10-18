//! Lexer, conditional analysis and parser must never panic or hang.
#![no_main]

use libfuzzer_sys::fuzz_target;
use rsim_diag::{MessageFormat, SourceMap};
use rsim_syntax::{Encoding, Options, Std, load_source, parse_file};

fuzz_target!(|data: &[u8]| {
    let mut sources = SourceMap::default();
    // Both readings of the same bytes.
    for encoding in [Encoding::Auto, Encoding::Latin1] {
        let (file, _) = load_source(&mut sources, "fuzz.vhd", data.to_vec(), encoding);
        for std in [Std::Vhdl2008, Std::Vhdl2019] {
            let opts = Options { std, defines: Vec::new() };
            let parsed = parse_file(file, sources.get(file), &opts);
            // Rendering exercises every span the parser produced.
            for d in &parsed.diags {
                rsim_diag::render(&sources, d, MessageFormat::Human, false);
            }
        }
    }
});

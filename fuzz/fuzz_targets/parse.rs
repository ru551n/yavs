//! Lexer, conditional analysis and parser must never panic or hang.
#![no_main]

use libfuzzer_sys::fuzz_target;
use yavs_diag::{MessageFormat, SourceMap};
use yavs_syntax::{Encoding, Options, Std, load_source, parse_file};

fuzz_target!(|data: &[u8]| {
    let mut sources = SourceMap::default();
    // Both readings of the same bytes.
    for encoding in [Encoding::Auto, Encoding::Latin1] {
        let (file, _) = load_source(&mut sources, "fuzz.vhd", data.to_vec(), encoding);
        for (std, psl) in [(Std::Vhdl2008, false), (Std::Vhdl2019, true)] {
            let opts = Options {
                std,
                defines: Vec::new(),
                psl,
            };
            let parsed = parse_file(file, sources.get(file), &opts);
            // Rendering exercises every span the parser produced.
            for d in &parsed.diags {
                yavs_diag::render(&sources, d, MessageFormat::Human, false);
            }
        }
    }
});

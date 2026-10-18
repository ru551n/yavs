# Decisions

## Waveform writer: wavefst (rsim-wave)

- **Choice:** the `wavefst` crate (0.4.0, MIT) for FST output, with `default-features = false`,
  features `writer` and `lz4` (pure Rust, no C compiler needed). No FST writing code of our own.
- **Why:** wellen is reader only. `fst-writer` 0.3.1 (BSD-3) has no hierarchy attributes, so it
  cannot carry VHDL variable/data types or enum tables, its real-value path is unused and its
  flush only works at a time step boundary (`todo!`). `wavefst` has enum tables, VHDL supplemental
  variable metadata (`CreateVar2`), aliases, reals, `flush()` that patches the header, and is
  checked against libfst in both directions.
- **Risks:** a young single-author crate. All use is behind `rsim-wave`'s own API (`WaveWriter`),
  so it can be swapped or vendored. rsim-wave's round-trip tests read every file back with wellen.
- **FST mapping:** std_logic family and bit are written as characters (`u x 0 1 z w l h -`) with
  the VHDL data type set, so viewers show `U`, `X`, ...; other enums are bit vectors with an FST
  enum table (literal names in original spelling); integers and time are two's complement vectors;
  reals are doubles; vector bounds go into the name (`data[7:0]`, `rev[0:3]`, left first, so
  direction is kept); records and arrays are scopes with one signal per sub-element.
- **Durability:** a block is flushed on the next time advance after `flush_interval` (default
  5 s) or `block_change_limit` changes, and the header is patched each time, so a killed run is
  readable up to the last block. SIGINT/SIGTERM only set a flag (`interrupt::install`); the kernel
  stops between time steps and calls `finish`, which is also done on drop.

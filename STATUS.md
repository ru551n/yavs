# Status

## logic (`crates/rsim-logic`)

Done ahead of M3: `StdULogic` with the IEEE 1164 tables (checked against the vendored package
body), `Logic<R>`/`LogicVec` packed vectors in two layouts (nibbles, bit-planes), `BitVec` fast
path, numeric_std add/sub/compare/shift/rotate/integer conversion with X propagation, exhaustive
and property tests, criterion benchmarks. Default layout is bit-planes; see
`crates/rsim-logic/LAYOUT.md`. Open: the user confirms the layout decision (docs/decisions.md).

# AGENTS.md: yavs-verilog

Read the root `AGENTS.md` first. This file is for an agent about to change `crates/yavs-verilog`.

## 1. Purpose and boundaries

- Verilog (IEEE 1364-2005) front end: preprocessor, lexer, parser, module analysis and detection of
  encrypted envelopes and SystemVerilog. M0 of the plan in #776: no elaboration, no simulation.
- Depends on nothing: diagnostics are plain data (`Diag`), rendered and registered by the caller.

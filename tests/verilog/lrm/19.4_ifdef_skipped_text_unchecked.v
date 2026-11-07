// LRM 19.4: text in a skipped group is not compiled; yavs does not read it either, so it may hold anything that is not a directive (known gap G7: the LRM wants the lexical conventions kept)
// units: a
// gap: G7 lexical errors in skipped text are not reported
// expect: none
`timescale 1ns/1ps
`ifdef NEVER
this is not verilog at all ; ; )(
`endif
module a;
endmodule

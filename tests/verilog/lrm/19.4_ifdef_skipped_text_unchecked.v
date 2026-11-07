// LRM 19.4: text in a skipped branch is not compiled, so it may hold anything that is not a directive
// units: a
// expect: none
`timescale 1ns/1ps
`ifdef NEVER
this is not verilog at all ; ; )(
`endif
module a;
endmodule

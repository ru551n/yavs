// LRM 19.3.1: a macro is used only after its `define
// expect: E4007 5:9 no such macro
`timescale 1ns/1ps
module m;
  wire [`NOT_DEFINED:0] w;
endmodule

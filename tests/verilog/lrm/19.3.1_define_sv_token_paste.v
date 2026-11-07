// LRM 19.3.1: the `` token paste is not in 1364-2005
// expect: E4003 6:8 token paste is SystemVerilog
`timescale 1ns/1ps
`define P(a) a``_x
module m;
  wire `P(w);
endmodule

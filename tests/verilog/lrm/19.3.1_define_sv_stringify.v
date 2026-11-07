// LRM 19.3.1: the `" construct is not in 1364-2005
// expect: E4003 6:20 stringify is SystemVerilog
`timescale 1ns/1ps
`define S(a) `"a`"
module m;
  initial $display(`S(x));
endmodule

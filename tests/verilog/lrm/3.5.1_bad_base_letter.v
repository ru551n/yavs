// LRM 3.5.1: the base is one of b, o, d, h; a quote before anything else is read as SystemVerilog
// expect: E4003 5:18 quote is not part of a 1364 constant
`timescale 1ns/1ps
module m;
  parameter p = 8'q12;
endmodule

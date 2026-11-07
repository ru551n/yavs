// LRM 4.3.1: the bounds of a range are constant expressions: a missing one is a syntax error
// expect: E4100 5:10 missing lsb
`timescale 1ns/1ps
module m;
  reg [7:] a;
endmodule

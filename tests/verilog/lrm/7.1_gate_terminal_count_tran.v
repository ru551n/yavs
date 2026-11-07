// LRM 7.1: tran has exactly two terminals
// expect: E4100 6:3 three terminals
`timescale 1ns/1ps
module m;
  wire a, b, c;
  tran t (a, b, c);
endmodule

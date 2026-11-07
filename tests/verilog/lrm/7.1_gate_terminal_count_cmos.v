// LRM 7.1: cmos has exactly four terminals
// expect: E4100 6:3 three terminals
`timescale 1ns/1ps
module m;
  wire o, i, n;
  cmos c (o, i, n);
endmodule

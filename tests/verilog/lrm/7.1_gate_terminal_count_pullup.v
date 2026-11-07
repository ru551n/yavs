// LRM 7.1: pullup has exactly one terminal
// expect: E4100 6:3 two terminals
`timescale 1ns/1ps
module m;
  wire a, b;
  pullup p (a, b);
endmodule

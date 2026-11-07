// LRM 4.3: a vector net may be scalared or vectored
// units: m
// expect: none
`timescale 1ns/1ps
module m;
  wire scalared [3:0] a;
  tri0 vectored [3:0] b;
endmodule

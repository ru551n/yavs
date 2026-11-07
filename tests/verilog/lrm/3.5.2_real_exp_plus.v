// LRM 3.5.2: the real constant 5.0e+3 is legal
// units: m
// expect: none
`timescale 1ns/1ps
module m;
  real r = 5.0e+3;
endmodule

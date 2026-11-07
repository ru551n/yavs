// LRM 3.5.2: the real constant 1.30e-2 is legal
// units: m
// expect: none
`timescale 1ns/1ps
module m;
  real r = 1.30e-2;
endmodule

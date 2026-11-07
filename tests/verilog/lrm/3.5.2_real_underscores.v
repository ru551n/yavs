// LRM 3.5.2: the real constant 236.123_763_e-12 is legal
// units: m
// expect: none
`timescale 1ns/1ps
module m;
  real r = 236.123_763_e-12;
endmodule

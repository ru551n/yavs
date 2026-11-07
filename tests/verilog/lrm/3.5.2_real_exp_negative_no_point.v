// LRM 3.5.2: the real constant 29E-2 is legal
// units: m
// expect: none
`timescale 1ns/1ps
module m;
  real r = 29E-2;
endmodule

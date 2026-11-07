// LRM 3.5.2: the real constant 0.1 is legal
// units: m
// expect: none
`timescale 1ns/1ps
module m;
  real r = 0.1;
endmodule

// LRM 3.5.2: the real constant 1.2E12 is legal
// units: m
// expect: none
`timescale 1ns/1ps
module m;
  real r = 1.2E12;
endmodule

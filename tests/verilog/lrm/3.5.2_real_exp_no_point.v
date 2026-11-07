// LRM 3.5.2: the real constant 23E10 is legal
// units: m
// expect: none
`timescale 1ns/1ps
module m;
  real r = 23E10;
endmodule

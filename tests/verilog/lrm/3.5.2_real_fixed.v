// LRM 3.5.2: the real constant 2394.26331 is legal
// units: m
// expect: none
`timescale 1ns/1ps
module m;
  real r = 2394.26331;
endmodule

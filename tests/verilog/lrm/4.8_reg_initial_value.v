// LRM 4.8: a variable may be given a value where it is declared
// units: m
// expect: none
`timescale 1ns/1ps
module m;
  reg r = 1'b0;
  integer i = 5;
  real x = 1.5;
endmodule

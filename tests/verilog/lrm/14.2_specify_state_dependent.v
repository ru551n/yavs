// LRM 14.2: a path may be conditional with if, and ifnone is the default
// units: m
// expect: none
`timescale 1ns/1ps
module m (a, b, q);
  input a, b;
  output q;
  specify
    if (b) (a => q) = 1;
    ifnone (a => q) = 2;
  endspecify
endmodule

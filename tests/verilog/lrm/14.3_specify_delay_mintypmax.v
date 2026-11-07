// LRM 14.3: a path delay value may be min:typ:max
// units: m
// expect: none
`timescale 1ns/1ps
module m (a, q);
  input a;
  output q;
  specify
    (a => q) = (1:2:3, 2:3:4);
  endspecify
endmodule

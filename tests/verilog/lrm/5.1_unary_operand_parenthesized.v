// LRM 5.1: a parenthesized operand of a unary operator is a primary
// units: m
// expect: none
`timescale 1ns/1ps
module m;
  wire [1:0] a;
  wire x = ~(|a);
  wire y = - (-a[0]);
  wire z = a[0] - -1;
endmodule

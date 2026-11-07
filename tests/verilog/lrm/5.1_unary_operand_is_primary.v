// LRM 5.1: the operand of a unary operator is a primary (A.8.3): a second unary operator needs parentheses; accepted with a warning (V18)
// units: m
// expect: W4208 7:14 unary operator on a unary operator
`timescale 1ns/1ps
module m;
  wire [1:0] a;
  wire x = ~ |a;
endmodule

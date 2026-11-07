// LRM 5.1.1: the modulus operator is illegal on real operands (known gap G10: operand types are not checked)
// units: m
// gap: G10 rules that depend on operand types are not checked
// expect: none
`timescale 1ns/1ps
module m;
  real r;
  initial r = 1.5 % 2.0;
endmodule

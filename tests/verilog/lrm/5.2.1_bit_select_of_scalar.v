// LRM 5.2.1: a bit-select of a scalar is illegal (known gap G10: operand types are not checked)
// units: m
// gap: G10 rules that depend on operand types are not checked
// expect: none
`timescale 1ns/1ps
module m;
  reg r;
  wire w;
  assign w = r[0];
endmodule

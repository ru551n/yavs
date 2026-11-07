// LRM 3.5.1: a minus before a constant is a unary operator, not part of the constant
// units: m
// expect: none
`timescale 1ns/1ps
module m;
  parameter p = -8'd5;
endmodule

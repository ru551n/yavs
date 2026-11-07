// LRM 3.5.1: the integer constant 256'h0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef is legal
// units: m
// expect: none
`timescale 1ns/1ps
module m;
  parameter p = 256'h0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef;
endmodule

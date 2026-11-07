// LRM 3.5.1: the integer constant 1_000_000 is legal
// units: m
// expect: none
`timescale 1ns/1ps
module m;
  parameter p = 1_000_000;
endmodule

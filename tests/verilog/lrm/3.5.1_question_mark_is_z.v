// LRM 3.5.1: the integer constant 4'b??01 is legal
// units: m
// expect: none
`timescale 1ns/1ps
module m;
  parameter p = 4'b??01;
endmodule

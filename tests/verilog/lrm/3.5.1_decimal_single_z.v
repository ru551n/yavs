// LRM 3.5.1: the integer constant 8'dz is legal
// units: m
// expect: none
`timescale 1ns/1ps
module m;
  parameter p = 8'dz;
endmodule

// LRM 3.5.1: the integer constant 8'SD100 is legal
// units: m
// expect: none
`timescale 1ns/1ps
module m;
  parameter p = 8'SD100;
endmodule

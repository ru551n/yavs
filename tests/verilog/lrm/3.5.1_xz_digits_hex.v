// LRM 3.5.1: the integer constant 12'hx_z? is legal
// units: m
// expect: none
`timescale 1ns/1ps
module m;
  parameter p = 12'hx_z?;
endmodule

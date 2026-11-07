// LRM 3.5.1: the integer constant 'hFF is legal
// units: m
// expect: none
`timescale 1ns/1ps
module m;
  parameter p = 'hFF;
endmodule

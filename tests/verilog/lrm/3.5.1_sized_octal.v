// LRM 3.5.1: the integer constant 9'o777 is legal
// units: m
// expect: none
`timescale 1ns/1ps
module m;
  parameter p = 9'o777;
endmodule

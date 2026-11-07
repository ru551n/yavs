// LRM 12.3: a non-ANSI header lists names and the body declares direction and type
// units: m
// expect: none
`timescale 1ns/1ps
module m (a, b, c);
  input a;
  input [3:0] b;
  output reg c;
endmodule

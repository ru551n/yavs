// LRM 19.3.1: a comment is white space, also between the formal arguments of a macro (V15: the list may span lines)
// units: m
// expect: none
`timescale 1ns/1ps
`define SUM( a /* first */ , b // second
  ) (a + b)
module m;
  wire [3:0] x = `SUM(1, 2);
endmodule

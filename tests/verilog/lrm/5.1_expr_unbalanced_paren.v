// LRM 5.1: parentheses balance
// expect: E4100 6:21 closing parenthesis missing
`timescale 1ns/1ps
module m;
  integer i;
  initial i = (1 + 2;
endmodule

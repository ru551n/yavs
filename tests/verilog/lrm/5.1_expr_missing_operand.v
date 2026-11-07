// LRM 5.1: a binary operator has two operands
// expect: E4100 6:19 operand missing
`timescale 1ns/1ps
module m;
  integer i;
  initial i = 1 + ;
endmodule

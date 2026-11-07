// LRM 5.5.1: an expression with an unsigned operand is unsigned: a signed -1 compares as 15
// expect: none
// output: 0 1
`timescale 1ns/1ps
module m;
  reg signed [3:0] n = -1;
  reg [3:0] u = 1;
  initial $display("%0d %0d", n < u, n < $signed(u));
endmodule

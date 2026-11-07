// LRM 4.3.1: the bounds of a range are any constant integers, negative ones included, and an untyped parameter keeps the sign of its value (12.2)
// expect: none
// output: 1000 0111 13 1 1
`timescale 1ns/1ps
module m;
  parameter [0:-3] cp = 4'h8;
  parameter n = -3;
  reg [-1:2] r = 4'b0111;
  reg [2:-4] w = 0;
  initial begin
    w[-4] = 1'b1;
    w[-2] = 1'b1;
    w[-1] = 1'b1;
    $display("%b %b %0d %b %b", cp, r, w[-1:-4], n < 0, w[n + 1]);
  end
endmodule

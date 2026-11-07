// LRM 3.5.1: the size of a constant is a non-zero unsigned decimal number
// expect: E4014 5:17 size 0
`timescale 1ns/1ps
module m;
  parameter p = 0'b1;
endmodule

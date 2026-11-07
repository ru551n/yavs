// LRM 3.5.1: a binary constant has only the digits 0, 1, x, z and ?
// expect: E4014 5:17 digit 2 in a binary constant
`timescale 1ns/1ps
module m;
  parameter p = 4'b0102;
endmodule

// LRM 3.5.1: a decimal constant has only the digits 0 to 9
// expect: E4014 5:17 hex digit in a decimal constant
`timescale 1ns/1ps
module m;
  parameter p = 4'dA;
endmodule

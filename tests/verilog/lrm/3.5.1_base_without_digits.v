// LRM 3.5.1: a based constant has at least one digit
// expect: E4014 5:17 no digits after the base
`timescale 1ns/1ps
module m;
  parameter p = 8'b;
endmodule

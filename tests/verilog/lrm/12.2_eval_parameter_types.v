// LRM 12.2: a parameter without a type or range takes the type of its value; one with a range keeps it
// expect: none
// output: 1010 11 44
`timescale 1ns/1ps
module m;
  parameter P = 4'b1010;
  parameter Q = P + 1;
  parameter [7:0] R = 300;
  initial $display("%b %0d %0d", P, Q, R);
endmodule

// LRM 5.1.12: >>> fills with the sign bit only when the expression is signed; an x shift amount makes the result x
// expect: none
// output: 11111100 01111100 01111100 xxxxxxxx
`timescale 1ns/1ps
module m;
  reg signed [7:0] s = -8;
  reg [7:0] u = 8'hf8;
  initial $display("%b %b %b %b", s >>> 1, u >>> 1, s >> 1, u << 1'bx);
endmodule

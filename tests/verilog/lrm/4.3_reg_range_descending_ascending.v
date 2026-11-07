// LRM 4.3: a vector range may ascend or descend
// units: m
// expect: none
`timescale 1ns/1ps
module m;
  reg [0:7] a;
  reg [7:0] b;
  reg [-4:3] c;
endmodule

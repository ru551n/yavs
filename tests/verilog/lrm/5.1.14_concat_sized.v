// LRM 5.1.14: sized constants, strings and replications stand in a concatenation
// units: m
// expect: none
`timescale 1ns/1ps
module m;
  reg [7:0] a;
  wire [24:0] w;
  assign w = {a, 1'b0, 8'hFF, "A", {2{a[0]}}};
endmodule

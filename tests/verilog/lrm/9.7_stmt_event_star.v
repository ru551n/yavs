// LRM 9.7: @* and @(*) are the implicit sensitivity list
// units: m
// expect: none
`timescale 1ns/1ps
module m;
  reg a, b;
  always @* a = b;
  always @(*) b = a;
endmodule

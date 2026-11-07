// LRM 9.9: initial and always are the procedural blocks
// units: m
// expect: none
`timescale 1ns/1ps
module m;
  reg clk;
  initial clk = 0;
  always #5 clk = ~clk;
endmodule

// LRM 15: timing checks are parsed; without XIL_TIMING they are accepted silently
// units: m
// expect: none
`timescale 1ns/1ps
module m (clk, d);
  input clk, d;
  specify
    $setup(d, posedge clk, 1);
    $hold(posedge clk, d, 1);
  endspecify
endmodule

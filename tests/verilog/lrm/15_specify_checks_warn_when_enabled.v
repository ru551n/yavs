// LRM 15: with XIL_TIMING defined the checks are still not evaluated: one warning per call says so
// yavs-test: args=-D XIL_TIMING
// units: m
// expect: W4015 1:1 timing checks are not evaluated
`timescale 1ns/1ps
module m (clk, d);
  input clk, d;
  specify
    $setup(d, posedge clk, 1);
  endspecify
endmodule

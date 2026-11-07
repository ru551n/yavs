// LRM 19.5: the <file> form of `include is not in 1364-2005; accepted as an extension (V5)
// yavs-test: args=-I inc
// units: m
// expect: none
`timescale 1ns/1ps
`include <width.vh>
module m;
  wire [`LRM_WIDTH-1:0] w;
endmodule

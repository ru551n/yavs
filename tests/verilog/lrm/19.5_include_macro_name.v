// LRM 19.5: the file name of an `include may come from a macro that expands to a string
// yavs-test: args=-I inc
// units: m
// expect: none
`timescale 1ns/1ps
`define HDR "width.vh"
`include `HDR
module m;
  wire [`LRM_WIDTH-1:0] w;
endmodule

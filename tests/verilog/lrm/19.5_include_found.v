// LRM 19.5: `include inserts the text of the named file
// yavs-test: args=-I inc
// units: m
// expect: none
`timescale 1ns/1ps
`include "width.vh"
module m;
  wire [`LRM_WIDTH-1:0] w;
endmodule

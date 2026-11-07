// LRM 19.3: a later `define of the same name replaces the earlier text
// units: m
// expect: none
`timescale 1ns/1ps
`define W 4
`define W 8
module m;
  wire [`W-1:0] w;
endmodule

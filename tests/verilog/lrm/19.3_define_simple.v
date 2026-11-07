// LRM 19.3: a text macro defined with `define is replaced where `NAME is used
// units: m
// expect: none
`timescale 1ns/1ps
`define W 8
module m;
  wire [`W-1:0] w;
endmodule

// LRM 19.3.1: a macro may have an empty text
// units: m
// expect: none
`timescale 1ns/1ps
`define NOTHING
module m;
  `NOTHING
  wire w;
endmodule

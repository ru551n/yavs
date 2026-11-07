// LRM 3.8: an attribute may precede a statement
// units: m
// expect: none
`timescale 1ns/1ps
module m;
  reg r;
  initial (* synthesis *) r = 1'b0;
endmodule

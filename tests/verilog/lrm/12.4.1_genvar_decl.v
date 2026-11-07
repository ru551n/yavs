// LRM 12.4.1: genvar declares generate loop variables
// units: m
// expect: none
`timescale 1ns/1ps
module m;
  genvar i, j;
endmodule

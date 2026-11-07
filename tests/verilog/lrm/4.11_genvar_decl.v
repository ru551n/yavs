// LRM 4.11: genvar declares generate loop variables
// units: m
// expect: none
`timescale 1ns/1ps
module m;
  genvar i, j;
endmodule

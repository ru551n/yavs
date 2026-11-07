// LRM 19.3: a macro text continues on the next line after a backslash at the end of a line
// units: m
// expect: none
`timescale 1ns/1ps
`define TWO_WIRES wire a; \
  wire b;
module m;
  `TWO_WIRES
endmodule

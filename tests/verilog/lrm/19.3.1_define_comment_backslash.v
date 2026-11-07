// LRM 19.3.1: a backslash at the end of a line comment inside a macro text still continues the macro (V15)
// units: m
// expect: none
`timescale 1ns/1ps
`define BOTH wire a; // first \
  wire b;
module m;
  `BOTH
endmodule

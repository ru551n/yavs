// LRM 19.3.1: the text of a macro may use other macros, expanded when it is used
// units: m
// expect: none
`timescale 1ns/1ps
`define A 1
`define B (`A + 1)
module m;
  parameter p = `B;
endmodule

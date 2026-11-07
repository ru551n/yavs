// LRM 19.3: a macro name inside a string is not expanded
// units: m
// expect: none
`timescale 1ns/1ps
`define W 8
module m;
  initial $display("`W");
endmodule

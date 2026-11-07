// LRM 19.3: a comma inside a string does not separate actual arguments
// units: m
// expect: none
`timescale 1ns/1ps
`define ID(x) x
module m;
  initial $display(`ID("a,b"));
endmodule

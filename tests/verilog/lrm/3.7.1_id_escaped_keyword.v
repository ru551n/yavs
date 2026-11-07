// LRM 3.7.1: an escaped keyword is an identifier, not the keyword
// units: m
// expect: none
`timescale 1ns/1ps
module m;
  wire \module ;
endmodule

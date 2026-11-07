// LRM 3.7: identifiers are case sensitive: a and A are different names
// units: m
// expect: none
`timescale 1ns/1ps
module m;
  wire a;
  wire A;
endmodule

// LRM 4.9.1: a net may be an array
// units: m
// expect: none
`timescale 1ns/1ps
module m;
  wire [7:0] arr [0:3];
endmodule

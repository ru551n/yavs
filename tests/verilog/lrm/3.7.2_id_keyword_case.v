// LRM 3.7.2: keywords are lower case: Module is an identifier
// units: m
// expect: none
`timescale 1ns/1ps
module m;
  wire Module;
  wire WIRE;
endmodule

// LRM 4.9: an array may have more than one dimension
// units: m
// expect: none
`timescale 1ns/1ps
module m;
  reg [7:0] mem [0:3][0:3];
  integer cnt [0:7];
endmodule

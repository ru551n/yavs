// LRM 3.5.1: underscores are legal in the size
// units: m
// expect: none
`timescale 1ns/1ps
module m;
  parameter p = 1_6'hFF;
endmodule

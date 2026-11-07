// LRM 3.5.1: the integer constant 16'hDeAd is legal
// units: m
// expect: none
`timescale 1ns/1ps
module m;
  parameter p = 16'hDeAd;
endmodule

// LRM 4.10.1: a parameter may be typed, signed, ranged, real or a string
// units: m
// expect: none
`timescale 1ns/1ps
module m;
  parameter a = 1, b = 2;
  parameter integer c = 3;
  parameter signed [7:0] d = -1;
  parameter real e = 2.5;
  parameter [8*3:1] f = "abc";
  parameter time g = 10;
  localparam h = a + b;
endmodule

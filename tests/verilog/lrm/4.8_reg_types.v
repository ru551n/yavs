// LRM 4.8: reg, integer, time, real and realtime declare variables
// units: m
// expect: none
`timescale 1ns/1ps
module m;
  reg r;
  reg signed [7:0] s;
  integer i, j;
  time t;
  real x;
  realtime y;
endmodule

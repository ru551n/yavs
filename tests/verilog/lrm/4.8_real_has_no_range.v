// LRM 4.8: integer, time, real and realtime variables have no range
// expect: E4100 5:8 range on real
`timescale 1ns/1ps
module m;
  real [3:0] r;
endmodule

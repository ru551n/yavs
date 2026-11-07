// LRM 4.8: integer, time, real and realtime variables have no range
// expect: E4100 5:11 range on integer
`timescale 1ns/1ps
module m;
  integer [3:0] i;
endmodule

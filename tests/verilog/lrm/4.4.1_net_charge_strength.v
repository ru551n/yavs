// LRM 4.4.1: a trireg may carry a charge strength
// units: m
// expect: none
`timescale 1ns/1ps
module m;
  trireg (small) t1;
  trireg (medium) t2;
  trireg (large) [3:0] t3;
endmodule

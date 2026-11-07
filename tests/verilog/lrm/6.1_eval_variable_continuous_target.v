// LRM 6.1: a continuous assignment drives a net, never a variable
// expect: none
// elab: E4507
`timescale 1ns/1ps
module m;
  reg r;
  assign r = 1;
endmodule

// LRM 4.2.1: a trireg with a drive strength takes a declaration assignment
// units: m
// expect: none
`timescale 1ns/1ps
module m;
  trireg (strong0, strong1) t = 1'b1;
endmodule

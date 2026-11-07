// LRM 4.11: instance parameters are overridden by position
// units: c,m
// expect: none
`timescale 1ns/1ps
module c #(parameter A = 1, B = 2) ();
endmodule
module m;
  c #(3, 4) u ();
endmodule

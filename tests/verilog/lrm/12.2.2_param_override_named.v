// LRM 12.2.2: instance parameters are overridden by name
// units: c,m
// expect: none
`timescale 1ns/1ps
module c #(parameter A = 1, B = 2) ();
endmodule
module m;
  c #(.B(4)) u ();
endmodule

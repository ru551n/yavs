// LRM 4.11: defparam overrides a parameter by hierarchical name
// units: c,m
// expect: none
`timescale 1ns/1ps
module c #(parameter A = 1) ();
endmodule
module m;
  c u ();
  defparam u.A = 5;
endmodule

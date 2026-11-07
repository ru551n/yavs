// LRM 3.8: an attribute may precede a module instance
// units: c,m
// expect: none
`timescale 1ns/1ps
module c;
endmodule
module m;
  (* keep_hierarchy *) c u ();
endmodule

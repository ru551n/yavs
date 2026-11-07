// LRM 3.8: an attribute may have a constant value
// units: m
// expect: none
`timescale 1ns/1ps
module m;
  (* keep = 1, mark = "x" *) wire w;
endmodule

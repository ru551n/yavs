// LRM 3.8: an attribute instance (* name *) may precede a declaration
// units: m
// expect: none
`timescale 1ns/1ps
module m;
  (* keep *) wire w;
endmodule

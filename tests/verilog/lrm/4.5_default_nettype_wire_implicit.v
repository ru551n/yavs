// LRM 4.5: by default an undeclared name on the left of a continuous assignment is an implicit one-bit wire
// units: m
// expect: none
`timescale 1ns/1ps
module m (output o, input a);
  assign missing = a;
  assign o = a;
endmodule

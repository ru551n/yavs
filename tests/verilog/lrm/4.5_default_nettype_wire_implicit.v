// LRM 4.5: by default an undeclared name in a continuous assignment is an implicit one-bit wire
// units: m
// expect: none
`timescale 1ns/1ps
module m (output o, input a);
  assign o = missing & a;
endmodule

// LRM 3.5.2: a decimal point has a digit on each side
// expect: E4100 5:13 no digit after the decimal point
`timescale 1ns/1ps
module m;
  real r = 9.;
endmodule

// LRM 3.5.2: a decimal point has a digit on each side
// expect: E4100 5:12 no digit before the decimal point
`timescale 1ns/1ps
module m;
  real r = .5;
endmodule

// LRM 14.2: specparam declares constants for use in path delays
// units: m
// expect: none
`timescale 1ns/1ps
module m (a, q);
  input a;
  output q;
  specify
    specparam tpd = 1.5, tmin = 1:2:3;
    (a => q) = tpd;
  endspecify
endmodule

// LRM 14.6.1: PATHPULSE$ specparams set the pulse limits of a path
// units: m
// expect: none
`timescale 1ns/1ps
module m (a, q);
  input a;
  output q;
  specify
    specparam PATHPULSE$a$q = (0.5, 1.5);
    (a => q) = 2;
  endspecify
endmodule

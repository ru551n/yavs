// LRM 6.1.4: the order of the two strengths is free
// units: m
// expect: none
`timescale 1ns/1ps
module m;
  wire a, b;
  assign (pull1, strong0) a = b;
  assign (strong0, pull1) b = a;
endmodule

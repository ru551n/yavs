// LRM 14.2.2: a parallel path connects one input to one output: (in => out) = delay
// units: m
// expect: none
`timescale 1ns/1ps
module m (a, q);
  input a;
  output q;
  specify
    (a => q) = 1;
  endspecify
endmodule

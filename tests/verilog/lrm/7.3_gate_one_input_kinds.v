// LRM 7.3: buf and not take any number of outputs followed by one input
// units: m
// expect: none
`timescale 1ns/1ps
module m;
  wire o1, o2, i;
  buf b1 (o1, i);
  not n1 (o1, o2, i);
endmodule

// LRM 12.1.2: an instance may be an array of instances with a range
// units: c,m
// expect: none
`timescale 1ns/1ps
module c (input a);
endmodule
module m;
  wire [3:0] x;
  c u [3:0] (x);
endmodule

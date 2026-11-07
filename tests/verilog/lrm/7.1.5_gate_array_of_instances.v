// LRM 7.1.5: a gate instance may be an array with a range
// units: m
// expect: none
`timescale 1ns/1ps
module m;
  wire [3:0] o, a, b;
  and g [3:0] (o, a, b);
endmodule

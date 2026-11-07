// LRM 4.6.1: tri is a net type
// units: m
// expect: none
`timescale 1ns/1ps
module m;
  tri n;
  tri [7:0] v;
endmodule

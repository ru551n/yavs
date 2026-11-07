// LRM 4.6.6: supply1 is a net type
// units: m
// expect: none
`timescale 1ns/1ps
module m;
  supply1 n;
  supply1 [7:0] v;
endmodule

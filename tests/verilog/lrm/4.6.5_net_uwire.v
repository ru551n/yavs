// LRM 4.6.5: uwire is a net type
// units: m
// expect: none
`timescale 1ns/1ps
module m;
  uwire n;
  uwire [7:0] v;
endmodule

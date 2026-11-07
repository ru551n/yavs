// LRM 4.2: supply0 is a net type
// units: m
// expect: none
`timescale 1ns/1ps
module m;
  supply0 n;
  supply0 [7:0] v;
endmodule

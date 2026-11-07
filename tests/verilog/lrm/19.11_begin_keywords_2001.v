// LRM 19.11: under "1364-2001" uwire is not a keyword and may name a net
// units: m
// expect: none
`timescale 1ns/1ps
`begin_keywords "1364-2001"
module m;
  wire uwire;
endmodule
`end_keywords

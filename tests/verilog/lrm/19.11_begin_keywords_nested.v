// LRM 19.11: the pairs nest, and the set of the outer pair returns at the inner end_keywords
// units: m
// expect: none
`timescale 1ns/1ps
`begin_keywords "1364-2001"
`begin_keywords "1364-2005"
`end_keywords
module m;
  wire uwire;
endmodule
`end_keywords

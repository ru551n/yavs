// LRM 19.11: under "1364-1995" the words added later (automatic, generate, signed, ...) are identifiers
// units: m
// expect: none
`timescale 1ns/1ps
`begin_keywords "1364-1995"
module m;
  wire automatic, generate, signed, localparam;
endmodule
`end_keywords

// LRM 3.7.1: an escaped identifier may hold a backtick, a quote and a slash: the preprocessor does not read them
// units: m
// expect: none
`timescale 1ns/1ps
module m;
  reg \`~!-_=+\|[]{};:'"",./<>? ;
  wire \a`b ;
endmodule

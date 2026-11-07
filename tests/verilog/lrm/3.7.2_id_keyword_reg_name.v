// LRM 3.7.2: a keyword of the language, here begin, cannot name a net
// expect: E4100 5:8 begin is a keyword
`timescale 1ns/1ps
module m;
  wire begin;
endmodule

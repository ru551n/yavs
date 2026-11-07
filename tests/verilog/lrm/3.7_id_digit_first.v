// LRM 3.7: a simple identifier does not start with a digit
// expect: E4014 5:8 1a is a malformed number
`timescale 1ns/1ps
module m;
  wire 1a;
endmodule

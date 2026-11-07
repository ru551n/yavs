// LRM 12.3.6: named and positional connections do not mix in one instance
// expect: E4100 8:11 named connection after a positional one
`timescale 1ns/1ps
module c (input a, input b);
endmodule
module m;
  wire x, y;
  c u (x, .b(y));
endmodule

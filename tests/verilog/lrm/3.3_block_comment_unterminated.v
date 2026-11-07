// LRM 3.3: a block comment ends at the first */; one that never ends is an error
// expect: E4014 6:3 unterminated block comment
`timescale 1ns/1ps
module m;
  wire w;
  /* never closed
endmodule

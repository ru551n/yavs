// LRM 4.2: a declaration ends with a semicolon
// expect: E4100 6:3 missing semicolon
`timescale 1ns/1ps
module m;
  wire a
  wire b;
endmodule

// LRM 14.1: a specify block ends with endspecify
// expect: E4100 9:1 endspecify missing
`timescale 1ns/1ps
module m (a, q);
  input a;
  output q;
  specify
    (a => q) = 1;
endmodule

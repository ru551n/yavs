// LRM 14.2.2: a path declaration has a delay
// expect: E4100 8:13 no delay
`timescale 1ns/1ps
module m (a, q);
  input a;
  output q;
  specify
    (a => q);
  endspecify
endmodule

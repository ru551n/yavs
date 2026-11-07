// LRM 14.3: a path delay with four values is not allowed
// expect: E4100 8:16 four delay values
`timescale 1ns/1ps
module m (a, q);
  input a;
  output q;
  specify
    (a => q) = (1, 2, 3, 4);
  endspecify
endmodule

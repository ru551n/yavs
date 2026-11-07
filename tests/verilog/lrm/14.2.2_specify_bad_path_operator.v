// LRM 14.2.2: a path uses => or *>
// expect: E4100 8:8 -> is not a path operator
`timescale 1ns/1ps
module m (a, q);
  input a;
  output q;
  specify
    (a -> q) = 1;
  endspecify
endmodule

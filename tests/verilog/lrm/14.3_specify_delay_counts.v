// LRM 14.3: a path delay has 1, 2, 3, 6 or 12 values
// units: m
// expect: none
`timescale 1ns/1ps
module m (a, q);
  input a;
  output q;
  specify
    (a => q) = 1;
    (a => q) = (1, 2);
    (a => q) = (1, 2, 3);
    (a => q) = (1, 2, 3, 4, 5, 6);
    (a => q) = (1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12);
  endspecify
endmodule

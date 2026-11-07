// LRM 7.1: a gate instantiation ends with a semicolon
// expect: E4100 7:3 missing semicolon
`timescale 1ns/1ps
module m;
  wire o, a, b;
  and g (o, a, b)
  wire c;
endmodule

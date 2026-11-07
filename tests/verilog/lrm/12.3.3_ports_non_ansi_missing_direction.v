// LRM 12.3.3: every port of the list has a direction declaration in the body
// expect: E4202 4:14 b has no direction
`timescale 1ns/1ps
module m (a, b);
  input a;
endmodule

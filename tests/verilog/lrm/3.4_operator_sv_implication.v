// LRM 3.4: -> in an expression is not a 1364 operator, only the event trigger statement
// expect: E4100 6:17 -> is not an expression operator
`timescale 1ns/1ps
module m;
  integer a, b, c;
  initial c = a -> b;
endmodule

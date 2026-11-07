// LRM 9.4: the condition of an if is in parentheses
// expect: E4100 6:14 condition not in parentheses
`timescale 1ns/1ps
module m;
  reg a;
  initial if a a = 0;
endmodule

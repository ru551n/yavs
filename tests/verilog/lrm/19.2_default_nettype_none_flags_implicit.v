// LRM 19.2: with `default_nettype none an undeclared name is an error, not an implicit wire (also 4.5)
// expect: E4201 6:14 missing is not declared
`timescale 1ns/1ps
`default_nettype none
module m (output o, input a);
  assign o = missing & a;
endmodule
`default_nettype wire

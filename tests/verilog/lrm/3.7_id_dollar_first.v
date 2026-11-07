// LRM 3.7: a simple identifier does not start with $
// expect: E4100 5:8 $a is a system name
`timescale 1ns/1ps
module m;
  wire $a;
endmodule

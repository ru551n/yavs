// LRM 4.11: a parameter has a default value
// expect: E4100 5:14 no default value
`timescale 1ns/1ps
module m;
  parameter a;
endmodule

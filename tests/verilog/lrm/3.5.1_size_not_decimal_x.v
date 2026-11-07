// LRM 3.5.1: the size is decimal digits only
// expect: E4100 5:19 x8 is an identifier followed by a constant
`timescale 1ns/1ps
module m;
  parameter p = x8'hFF;
endmodule

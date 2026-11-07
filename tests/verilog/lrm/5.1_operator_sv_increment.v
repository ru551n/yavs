// LRM 5.1: ++ is not a 1364 operator
// expect: E4003 6:12 ++ is SystemVerilog
`timescale 1ns/1ps
module m;
  integer i;
  initial i++;
endmodule

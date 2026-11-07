// LRM 3.4: += is not a 1364 operator
// expect: E4003 6:13 += is SystemVerilog
`timescale 1ns/1ps
module m;
  integer i;
  initial i += 1;
endmodule

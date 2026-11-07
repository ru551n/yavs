// LRM 3.5.1: the unsized fill literals '0 and '1 are SystemVerilog
// expect: E4003 6:15 '1 is SystemVerilog
`timescale 1ns/1ps
module m;
  reg [3:0] r;
  initial r = '1;
endmodule

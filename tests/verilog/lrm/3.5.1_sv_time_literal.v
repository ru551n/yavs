// LRM 3.5.1: a time literal such as 1ns is SystemVerilog
// expect: E4003 5:12 time literal is SystemVerilog
`timescale 1ns/1ps
module m;
  initial #1ns;
endmodule

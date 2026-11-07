// LRM 3.7.2: enum is SystemVerilog
// expect: E4003 5:3 enum is SystemVerilog
`timescale 1ns/1ps
module m;
  enum {A, B} e;
endmodule

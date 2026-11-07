// LRM 9.2: compound assignment operators are SystemVerilog
// expect: E4003 6:13 += is SystemVerilog
`timescale 1ns/1ps
module m;
  integer a;
  initial a += 1;
endmodule

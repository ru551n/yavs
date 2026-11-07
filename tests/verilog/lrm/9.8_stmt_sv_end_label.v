// LRM 9.8: a label after end is SystemVerilog
// expect: E4003 8:7 end label is SystemVerilog
`timescale 1ns/1ps
module m;
  reg a;
  initial begin : b
    a = 1;
  end : b
endmodule

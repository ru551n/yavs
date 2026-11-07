// LRM 9.9: always_ff is SystemVerilog
// expect: E4003 6:3 always_ff is SystemVerilog
`timescale 1ns/1ps
module m;
  reg a, clk;
  always_ff @(posedge clk) a <= 1;
endmodule

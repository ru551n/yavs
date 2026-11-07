// LRM 12.4.1: the name of a generate block does not conflict with another declaration
// expect: E4203 7:41 block name is a reg
`timescale 1ns/1ps
module m;
  genvar i;
  reg a;
  for (i = 0; i < 2; i = i + 1) begin : a
  end
endmodule

// LRM 4.11: a block name and a net of the same scope share one name space
// expect: E4203 7:8 block name again
`timescale 1ns/1ps
module m;
  initial begin : b
  end
  wire b;
endmodule

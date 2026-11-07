// LRM 4.6.5: a uwire may have one driver per bit
// expect: none
// output: 10
`timescale 1ns/1ps
module m;
  uwire [1:0] w;
  assign w[0] = 1'b0;
  assign w[1] = 1'b1;
  initial #1 $display("%b", w);
endmodule

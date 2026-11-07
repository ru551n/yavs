// LRM 9.5.1: casez treats z and ? bits on either side as don't care; x still has to match
// expect: none
// output: 1 2 0
`timescale 1ns/1ps
module m;
  reg [3:0] s;
  reg [1:0] r;
  always @(s)
    casez (s)
      4'b1??0: r = 1;
      4'b01x?: r = 2;
      default: r = 0;
    endcase
  initial begin
    s = 4'b1010;
    #1 $write("%0d ", r);
    s = 4'b01x1;
    #1 $write("%0d ", r);
    s = 4'b0101;
    #1 $display("%0d", r);
  end
endmodule

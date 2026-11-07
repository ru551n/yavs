// LRM 6.1: a continuous assignment follows its operands; a net declaration assignment is one
// expect: none
// output: 4 9
`timescale 1ns/1ps
module m;
  reg [3:0] a;
  wire [3:0] w = a + 1;
  wire [3:0] v;
  assign v = w;
  initial begin
    a = 3;
    #1 $write("%0d ", v);
    a = 8;
    #1 $display("%0d", v);
  end
endmodule

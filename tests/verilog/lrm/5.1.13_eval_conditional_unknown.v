// LRM 5.1.13: an x condition of ?: combines both values bit by bit: equal bits stay, the others are x
// expect: none
// output: 1xx0
`timescale 1ns/1ps
module m;
  reg c;
  initial begin
    c = 1'bx;
    $display("%b", c ? 4'b1100 : 4'b1010);
  end
endmodule

// LRM 5.2.1: a bit-select outside the vector reads as x, a part-select partly outside reads x there
// expect: none
// output: x x1
`timescale 1ns/1ps
module m;
  reg [3:0] a = 4'b1001;
  integer i = 7;
  initial $display("%b %b", a[i], a[4:3]);
endmodule

// LRM 5.1.8: == is x when unknown bits make it ambiguous and 0 when known bits differ; === compares x and z exactly
// expect: none
// output: x 0 1 0
`timescale 1ns/1ps
module m;
  initial $display("%b %b %b %b", 4'b1x00 == 4'b1x00, 4'b1x00 == 4'b0x00, 4'b1x00 === 4'b1x00, 4'b1z00 === 4'b1x00);
endmodule

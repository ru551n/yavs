// LRM 12.4.1: the block of a generate loop may be unnamed
// units: m
// expect: none
`timescale 1ns/1ps
module m;
  genvar i;
  generate
    for (i = 0; i < 2; i = i + 1) begin
      wire w;
    end
  endgenerate
endmodule

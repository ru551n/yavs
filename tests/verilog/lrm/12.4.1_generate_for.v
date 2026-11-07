// LRM 12.4.1: a generate for loop uses a genvar and a named or unnamed block
// units: m
// expect: none
`timescale 1ns/1ps
module m;
  genvar i;
  wire [3:0] w;
  generate
    for (i = 0; i < 4; i = i + 1) begin : g
      assign w[i] = 1'b0;
    end
  endgenerate
endmodule

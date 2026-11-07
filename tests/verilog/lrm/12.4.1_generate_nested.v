// LRM 12.4.1: generate constructs nest
// units: m
// expect: none
`timescale 1ns/1ps
module m;
  genvar i, j;
  generate
    for (i = 0; i < 2; i = i + 1) begin : o
      for (j = 0; j < 2; j = j + 1) begin : n
        wire w;
      end
    end
  endgenerate
endmodule

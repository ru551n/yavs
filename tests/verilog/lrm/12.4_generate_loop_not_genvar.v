// LRM 12.4: the variable of a generate for loop is a genvar
// expect: E4204 7:10 i is not a genvar
`timescale 1ns/1ps
module m;
  integer i;
  generate
    for (i = 0; i < 2; i = i + 1) begin : g
    end
  endgenerate
endmodule

// LRM 12.4.1: two loop generate constructs do not use the same block name
// expect: E4203 8:43 block name again
`timescale 1ns/1ps
module m;
  genvar i;
  for (i = 1; i < 5; i = i + 1) begin : a
  end
  for (i = 10; i < 15; i = i + 1) begin : a
  end
endmodule

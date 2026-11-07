// LRM 12.4.1: the initialization and the iteration of a loop assign the same genvar
// expect: E4100 5:22 other genvar
module m;
  genvar i, j;
  for (i = 0; i < 2; j = j + 1) begin : g
  end
endmodule

// LRM 12.4.1: two nested loop generate constructs do not use the same genvar
// expect: E4100 6:24 genvar reused
module m;
  genvar i;
  for (i = 0; i < 2; i = i + 1) begin : a
    for (i = 0; i < 2; i = i + 1) begin : b
    end
  end
endmodule

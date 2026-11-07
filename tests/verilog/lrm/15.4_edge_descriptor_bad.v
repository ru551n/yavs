// LRM 15.4: 11 is not an edge descriptor
// expect: E4100 5:20 not a transition
module m (input d, clk);
  specify
    $setup(d, edge[11] clk, 1);
  endspecify
endmodule

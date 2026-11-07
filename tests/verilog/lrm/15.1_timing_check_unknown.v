// LRM 15.1: only the twelve timing checks of 15.1 appear in a specify block
// expect: E4100 5:5 not a timing check
module m (input d, clk);
  specify
    $bogus(d, clk, 1);
  endspecify
endmodule

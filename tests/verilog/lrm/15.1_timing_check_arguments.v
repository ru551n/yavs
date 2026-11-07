// LRM 15.1: $setup takes a data event, a reference event, a limit and an optional notifier
// expect: E4100 5:5 two arguments
module m (input d, clk);
  specify
    $setup(d, posedge clk);
  endspecify
endmodule

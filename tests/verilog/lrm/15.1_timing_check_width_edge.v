// LRM 15.1: the first argument of $width and $period is a controlled event with an edge
// expect: E4100 5:5 no edge
module m (input clk);
  specify
    $width(clk, 5);
  endspecify
endmodule

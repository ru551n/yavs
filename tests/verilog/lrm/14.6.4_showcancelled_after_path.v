// LRM 14.6.4: showcancelled names an output before any path declaration uses it
// expect: E4100 6:19 output already in a path
module m (input a, output q);
  specify
    (a => q) = 1;
    showcancelled q;
  endspecify
endmodule

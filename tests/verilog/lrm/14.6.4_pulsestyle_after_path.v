// LRM 14.6.4: a pulse style names an output before any path declaration uses it
// expect: E4100 6:24 output already in a path
module m (input a, output q);
  specify
    (a => q) = 1;
    pulsestyle_onevent q;
  endspecify
endmodule

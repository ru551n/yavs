// LRM 7.6: a bidirectional pass switch takes no drive strength
// expect: E4100 5:11 strength on tranif
module m;
  wire a, b, c;
  tranif1 (strong0, strong1) t (a, b, c);
endmodule

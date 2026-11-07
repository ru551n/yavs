// LRM 7.4: a three-state gate takes at most three delays
// expect: E4100 5:10 four delays
module m;
  wire o, a, e;
  bufif1 #(1, 2, 3, 4) g (o, a, e);
endmodule

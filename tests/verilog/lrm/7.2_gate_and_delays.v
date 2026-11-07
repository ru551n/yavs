// LRM 7.2: an n-input gate takes at most two delays
// expect: E4100 5:7 three delays
module m;
  wire o, a, b;
  and #(1, 2, 3) g (o, a, b);
endmodule

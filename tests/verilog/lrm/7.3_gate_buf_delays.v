// LRM 7.3: a buf or not gate takes at most two delays
// expect: E4100 5:7 three delays
module m;
  wire o, a;
  buf #(1, 2, 3) g (o, a);
endmodule

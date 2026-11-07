// LRM 7.6: tranif1 takes at most two delays
// expect: E4100 5:11 three delays
module m;
  wire a, b, c;
  tranif1 #(1, 2, 3) t (a, b, c);
endmodule

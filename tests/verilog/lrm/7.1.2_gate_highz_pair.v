// LRM 7.1.2: (highz0, highz1) is an invalid gate strength
// expect: E4100 5:8 both highz
module m;
  wire o, a, b;
  and (highz0, highz1) g (o, a, b);
endmodule

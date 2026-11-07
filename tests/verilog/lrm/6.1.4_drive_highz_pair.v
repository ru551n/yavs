// LRM 6.1.4: (highz0, highz1) and (highz1, highz0) are illegal drive strengths
// expect: E4100 5:11 both highz
module m;
  wire a, b;
  assign (highz1, highz0) a = b;
endmodule

// LRM 7.7: a cmos switch takes no drive strength
// expect: E4100 5:8 strength on cmos
module m;
  wire o, i, n, p;
  cmos (strong0, strong1) g (o, i, n, p);
endmodule

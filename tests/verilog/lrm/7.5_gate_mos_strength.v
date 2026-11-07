// LRM 7.5: a MOS switch takes no drive strength
// expect: E4100 5:8 strength on nmos
module m;
  wire o, i, c;
  nmos (strong0, strong1) g (o, i, c);
endmodule

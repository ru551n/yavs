// LRM 7.8: a pullup takes a strength1, a pulldown a strength0
// expect: E4100 5:10 strength0 on pullup
module m;
  wire a;
  pullup (strong0) (a);
endmodule

// LRM 6.1.4: a drive strength is one strength0 and one strength1
// expect: E4100 5:9 two strength0
module m;
  wire a = 1'b1;
  wire (strong0, weak0) b = a;
endmodule

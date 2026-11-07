// LRM 7.6: tran and rtran take no delay
// expect: E4100 5:8 delay on tran
module m;
  wire a, b;
  tran #1 t (a, b);
endmodule

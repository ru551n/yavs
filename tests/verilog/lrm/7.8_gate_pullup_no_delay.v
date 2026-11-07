// LRM 7.8: pullup and pulldown take no delay
// expect: E4100 5:10 delay on pullup
module m;
  wire a;
  pullup #1 (a);
endmodule

// LRM 3.5.1: a plus or minus between the base and the number is illegal (8 'd -6)
// expect: E4014 4:17 sign after base
module m;
  parameter p = 8 'd -6;
endmodule

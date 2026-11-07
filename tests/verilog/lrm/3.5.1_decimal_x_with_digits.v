// LRM 3.5.1: x or z in a decimal constant stands alone (the whole value)
// expect: E4014 5:17 x mixed with decimal digits
`timescale 1ns/1ps
module m;
  parameter p = 8'd1x;
endmodule

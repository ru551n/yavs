// LRM 3.5.1: an underscore may not be the first character of the digits
// expect: E4014 5:17 underscore as first digit
`timescale 1ns/1ps
module m;
  parameter p = 8'b_1010;
endmodule

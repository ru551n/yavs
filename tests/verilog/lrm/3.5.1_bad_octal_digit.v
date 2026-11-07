// LRM 3.5.1: an octal constant has only the digits 0 to 7, x, z and ?
// expect: E4014 5:17 digit 8 in an octal constant
`timescale 1ns/1ps
module m;
  parameter p = 6'o78;
endmodule

// LRM 3.5.1: a hexadecimal constant has only the digits 0 to 9, a to f, x, z and ?
// expect: E4014 5:17 digit G in a hexadecimal constant
`timescale 1ns/1ps
module m;
  parameter p = 8'hG1;
endmodule

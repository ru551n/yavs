// LRM 3.7.1: an escaped identifier has at least one character
// expect: E4014 5:8 empty escaped identifier
`timescale 1ns/1ps
module m;
  wire \ ;
endmodule

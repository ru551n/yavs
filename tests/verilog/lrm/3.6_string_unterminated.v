// LRM 3.6: a string ends at its closing quote
// expect: E4014 5:20 unterminated string
`timescale 1ns/1ps
module m;
  initial $display("never closed);
endmodule

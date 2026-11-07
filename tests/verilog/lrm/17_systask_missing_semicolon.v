// LRM 17: a system task enable ends with a semicolon
// expect: E4100 6:1 missing semicolon
`timescale 1ns/1ps
module m;
  initial $display("x")
endmodule

// LRM 9.2: a statement ends with a semicolon
// expect: E4100 8:5 missing semicolon
`timescale 1ns/1ps
module m;
  reg a;
  initial begin
    a = 1
    a = 0;
  end
endmodule

// LRM 9.8.1: begin ends with end
// expect: E4100 8:1 end missing
`timescale 1ns/1ps
module m;
  reg a;
  initial begin
    a = 1;
endmodule

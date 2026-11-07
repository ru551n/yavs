// LRM 9.8: fork and join run statements in parallel
// units: m
// expect: none
`timescale 1ns/1ps
module m;
  reg a;
  initial fork
    a = 1;
    #1 a = 0;
  join
endmodule

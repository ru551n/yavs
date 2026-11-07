// LRM 3.6: the empty string is legal
// units: m
// expect: none
`timescale 1ns/1ps
module m;
  initial $display("");
endmodule

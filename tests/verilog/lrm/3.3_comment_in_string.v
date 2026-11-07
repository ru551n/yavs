// LRM 3.3: comment markers inside a string are string text
// units: m
// expect: none
`timescale 1ns/1ps
module m;
  initial $display("// not /* a comment */");
endmodule

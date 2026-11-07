// LRM 3.6.3: a backslash before the newline continues a string on the next line (recalled: tools differ)
// units: m
// expect: none
`timescale 1ns/1ps
module m;
  initial $display("one\
two");
endmodule

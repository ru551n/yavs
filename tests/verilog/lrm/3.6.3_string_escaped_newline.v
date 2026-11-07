// LRM 3.6.3: a backslash before the newline continues a string; 1364-2005 lists no such escape and keeps a string on one line, other tools accept it, so it is accepted (V9)
// units: m
// expect: none
`timescale 1ns/1ps
module m;
  initial $display("one\
two");
endmodule

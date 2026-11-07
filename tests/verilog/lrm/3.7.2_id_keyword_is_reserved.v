// LRM 3.7.2: a keyword cannot be used as a simple identifier
// expect: E4100 5:8 module is a keyword
`timescale 1ns/1ps
module m;
  wire module;
endmodule

// LRM 7.1: a not gate has at least an output and an input
// expect: E4100 6:3 one terminal
`timescale 1ns/1ps
module m;
  wire o;
  not n (o);
endmodule

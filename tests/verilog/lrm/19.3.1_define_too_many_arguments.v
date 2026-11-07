// LRM 19.3.1: a macro call gives exactly as many actual arguments as the definition has formals
// expect: E4008 6:17 two arguments for one formal
`timescale 1ns/1ps
`define ID(a) a
module m;
  parameter p = `ID(1, 2);
endmodule

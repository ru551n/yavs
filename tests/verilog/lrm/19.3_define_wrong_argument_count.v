// LRM 19.3: a macro call gives exactly as many actual arguments as the definition has formals
// expect: E4008 6:17 one argument for two formals
`timescale 1ns/1ps
`define ADD(a, b) ((a) + (b))
module m;
  parameter p = `ADD(1);
endmodule

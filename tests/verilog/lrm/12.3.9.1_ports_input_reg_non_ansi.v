// LRM 12.3.9.1: a non-ANSI input declaration cannot say reg either
// expect: E4202 5:13 input port declared reg
`timescale 1ns/1ps
module m (a);
  input reg a;
endmodule

// LRM 12.3.9.1: an input port is a net: a later reg declaration of its name is an error too
// expect: E4202 6:7 input port redeclared reg
`timescale 1ns/1ps
module m (a);
  input a;
  reg a;
endmodule

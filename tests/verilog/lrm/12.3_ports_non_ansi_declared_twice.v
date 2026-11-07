// LRM 12.3: a port has one direction declaration
// expect: E4202 6:10 a declared twice
`timescale 1ns/1ps
module m (a);
  input a;
  output a;
endmodule

// LRM 12.3.2: a port name is listed once in the header
// expect: E4202 4:14 a listed twice
`timescale 1ns/1ps
module m (a, a);
  input a;
endmodule

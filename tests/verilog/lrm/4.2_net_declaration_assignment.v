// LRM 4.2: a net may be assigned where it is declared
// units: m
// expect: none
`timescale 1ns/1ps
module m;
  wire a = 1'b1, b = 1'b0;
endmodule

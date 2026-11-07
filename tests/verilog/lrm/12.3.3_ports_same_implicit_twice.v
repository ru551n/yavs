// LRM 12.3.3: the same implicit port listed twice is legal: the two ports are tied together
// units: m
// expect: none
`timescale 1ns/1ps
module m (a, a);
  input a;
endmodule

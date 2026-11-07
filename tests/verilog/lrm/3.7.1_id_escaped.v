// LRM 3.7.1: an escaped identifier runs to white space and may hold any printable character
// units: m
// expect: none
`timescale 1ns/1ps
module m;
  wire \bus+index ;
  wire \a*b ;
endmodule

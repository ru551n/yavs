// LRM 7.1.2: (highz1, strong0) makes an open collector gate
// units: m
// expect: none
`timescale 1ns/1ps
module m;
  wire o, a, b;
  nor (highz1, strong0) n1 (o, a, b);
endmodule

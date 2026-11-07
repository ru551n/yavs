// LRM 6.1.2: a continuous assignment with drive strength and delay
// units: m
// expect: none
`timescale 1ns/1ps
module m;
  wire a, b;
  assign (strong1, weak0) #3 a = b;
  assign #(1, 2) b = a, a = b;
endmodule

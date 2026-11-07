// LRM 7.1.2: a gate may carry a drive strength and a delay, strength first
// units: m
// expect: none
`timescale 1ns/1ps
module m;
  wire o, a, b;
  and (strong0, weak1) #(1, 2) g (o, a, b);
  nand (pull1, supply0) g2 (o, a, b);
endmodule

// LRM 7.8: pullup and pulldown take one output terminal
// units: m
// expect: none
`timescale 1ns/1ps
module m;
  wire a, b;
  pullup p1 (a);
  pulldown (b);
  pullup (strong1) p2 (b);
endmodule

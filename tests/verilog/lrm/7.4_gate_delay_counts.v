// LRM 7.4: the delays of a three-state gate are one, two or three
// units: m
// expect: none
`timescale 1ns/1ps
module m;
  wire o, a, e;
  bufif1 #1 g1 (o, a, e);
  bufif1 #(1, 2) g2 (o, a, e);
  bufif1 #(1, 2, 3) g3 (o, a, e);
endmodule

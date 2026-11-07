// LRM 7.2: a gate instance has an output followed by inputs; the instance name is optional
// units: m
// expect: none
`timescale 1ns/1ps
module m;
  wire o, a, b, c;
  and g1 (o, a, b);
  or (o, a, b, c);
  xor g2 (o, a, b), g3 (o, b, c);
  nand #1 (o, a, b);
  nor #(1, 2) g4 (o, a, b);
  xnor (o, a, b);
endmodule

// LRM 8.6: a UDP is instantiated like a gate, with an optional name and delay
// units: p,m
// expect: none
`timescale 1ns/1ps
primitive p (output o, input a);
  table
    0 : 1;
    1 : 0;
  endtable
endprimitive
module m;
  wire o, a;
  p u1 (o, a);
  p #2 (o, a);
  p (o, a);
endmodule

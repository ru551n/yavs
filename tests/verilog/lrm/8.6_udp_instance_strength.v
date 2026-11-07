// LRM 8.6: a UDP instance may carry a drive strength and a delay
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
  p (strong0, weak1) #2 u (o, a);
endmodule

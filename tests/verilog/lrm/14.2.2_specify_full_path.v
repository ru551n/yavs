// LRM 14.2.2: a full path connects lists of inputs to lists of outputs with *>
// units: m
// expect: none
`timescale 1ns/1ps
module m (a, b, p, q);
  input a, b;
  output p, q;
  specify
    (a, b *> p, q) = 2;
  endspecify
endmodule

// LRM 7.1: an and gate has an output and at least one input
// expect: E4100 6:3 one terminal
`timescale 1ns/1ps
module m;
  wire o;
  and g (o);
endmodule

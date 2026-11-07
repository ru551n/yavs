// LRM 7.1: a gate terminal is not left empty
// expect: E4100 6:3 empty terminal
`timescale 1ns/1ps
module m;
  wire o, a;
  and g (o, , a);
endmodule

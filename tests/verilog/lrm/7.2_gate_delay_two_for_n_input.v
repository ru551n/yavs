// LRM 7.2: min:typ:max delays are legal in a gate delay
// units: m
// expect: none
`timescale 1ns/1ps
module m;
  wire o, a, b;
  and #(1:2:3, 2:3:4) g (o, a, b);
endmodule

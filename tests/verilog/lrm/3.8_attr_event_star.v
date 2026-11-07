// LRM 3.8: (*) and @(*) are event-control star forms, not attributes
// units: m
// expect: none
`timescale 1ns/1ps
module m;
  reg a, b;
  always @(*) a = b;
  always @ (*) b = a;
endmodule

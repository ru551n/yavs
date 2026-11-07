// LRM 7.1: tran, rtran take two inout terminals; tranif forms add a control
// units: m
// expect: none
`timescale 1ns/1ps
module m;
  wire a, b, c;
  tran t1 (a, b);
  rtran t2 (a, b);
  tranif0 t3 (a, b, c);
  tranif1 t4 (a, b, c);
  rtranif0 t5 (a, b, c);
  rtranif1 t6 (a, b, c);
endmodule

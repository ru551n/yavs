// LRM 4.2: a net declaration may carry a delay of one, two or three values
// units: m
// expect: none
`timescale 1ns/1ps
module m;
  wire #5 a;
  wire #(1, 2) b;
  wire #(1, 2, 3) c;
  wire #(1:2:3) d;
endmodule

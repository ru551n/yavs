// LRM 5.3: a delay expression may be min:typ:max
// units: m
// expect: none
`timescale 1ns/1ps
module m;
  wire w;
  assign #(1:2:3) w = 1'b0;
endmodule

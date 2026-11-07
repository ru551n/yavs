// LRM 19.2: with `default_nettype none declared names are fine
// units: m
// expect: none
`timescale 1ns/1ps
`default_nettype none
module m (output o, input a);
  wire t;
  assign t = a;
  assign o = t;
endmodule
`default_nettype wire

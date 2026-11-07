// LRM 4.2: a net may be signed
// units: m
// expect: none
`timescale 1ns/1ps
module m;
  wire signed [7:0] s;
endmodule

// LRM 12.4: a generate if with one item needs no begin and end
// units: m
// expect: none
`timescale 1ns/1ps
module m;
  generate
    if (1) wire w;
  endgenerate
endmodule

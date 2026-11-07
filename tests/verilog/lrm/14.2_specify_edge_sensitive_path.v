// LRM 14.2: an edge-sensitive path names an edge and a data source
// units: m
// expect: none
`timescale 1ns/1ps
module m (clk, d, q);
  input clk, d;
  output q;
  specify
    (posedge clk => (q +: d)) = 1;
    (negedge clk *> (q -: d)) = 2;
  endspecify
endmodule

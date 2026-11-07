// LRM 15.4: an edge-control specifier lists the descriptors 01, 10, 0x, 1x, x0, x1 and the same with z
// units: m
// expect: none
`timescale 1ns/1ps
module m (input d, clk);
  specify
    $setup(d, edge[01, 10, 0x, x1, 1z, z0] clk, 1);
  endspecify
endmodule

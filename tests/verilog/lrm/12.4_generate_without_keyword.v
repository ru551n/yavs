// LRM 12.4: a generate loop or conditional may appear in a module without generate and endgenerate (1364-2005)
// units: m
// expect: none
`timescale 1ns/1ps
module m;
  genvar i;
  for (i = 0; i < 2; i = i + 1) begin : g
    wire w;
  end
  if (1) begin : h
    wire x;
  end
endmodule

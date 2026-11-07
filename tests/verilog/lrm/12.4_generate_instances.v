// LRM 12.4: a generate block may hold instances and procedural blocks
// units: c,m
// expect: none
`timescale 1ns/1ps
module c;
endmodule
module m;
  genvar i;
  generate
    for (i = 0; i < 2; i = i + 1) begin : g
      c u ();
      reg r;
      always @(r) r = ~r;
    end
  endgenerate
endmodule

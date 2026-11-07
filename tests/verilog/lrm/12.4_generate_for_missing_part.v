// LRM 12.4: the loop header has an initialisation, a condition and a step
// expect: E4100 7:22 step missing
`timescale 1ns/1ps
module m;
  genvar i;
  generate
    for (i = 0; i < 2) begin : g
    end
  endgenerate
endmodule

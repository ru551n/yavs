// LRM 12.4: a generate block may declare a localparam
// units: m
// expect: none
`timescale 1ns/1ps
module m;
  if (1) begin : g
    localparam p = 1;
  end
endmodule

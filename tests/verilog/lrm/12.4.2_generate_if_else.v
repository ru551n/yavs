// LRM 12.4.2: a generate if selects between blocks by a constant expression
// units: m
// expect: none
`timescale 1ns/1ps
module m;
  parameter P = 1;
  generate
    if (P) begin : a
      wire x;
    end else begin : b
      wire y;
    end
  endgenerate
endmodule

// LRM 12.4.1: the initialization of a loop does not reference its genvar (known gap G4: not checked)
// units: m
// gap: G4 a genvar in its own initialization is accepted
// expect: none
`timescale 1ns/1ps
module m;
  genvar i;
  for (i = i; i < 2; i = i + 1) begin : g
  end
endmodule

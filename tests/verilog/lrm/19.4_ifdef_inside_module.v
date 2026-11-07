// LRM 19.4: a conditional may stand between the items of a module
// units: m
// expect: none
`timescale 1ns/1ps
module m;
`ifdef X
  wire a;
`else
  wire b;
`endif
  wire c;
endmodule

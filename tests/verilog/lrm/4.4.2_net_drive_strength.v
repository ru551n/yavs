// LRM 4.4.2: a net declaration with an assignment may carry a drive strength
// units: m
// expect: none
`timescale 1ns/1ps
module m;
  wire (strong1, weak0) w = 1'b1;
  wire (pull0, supply1) v = 1'b0;
endmodule

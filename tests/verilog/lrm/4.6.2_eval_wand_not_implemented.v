// LRM 4.6.2: wired-AND nets are not implemented in Verilog elaboration
// expect: none
// elab: E4501
`timescale 1ns/1ps
module m;
  wand w;
  assign w = 1'b0;
endmodule

// LRM 9.2: a procedural assignment writes a variable, never a net
// expect: none
// elab: E4507
`timescale 1ns/1ps
module m;
  wire w;
  initial w = 1;
endmodule

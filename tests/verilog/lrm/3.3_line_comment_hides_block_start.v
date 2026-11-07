// LRM 3.3: inside a line comment /* has no meaning
// units: m
// expect: none
`timescale 1ns/1ps
module m;
  // /* not a block comment
  wire w;
endmodule

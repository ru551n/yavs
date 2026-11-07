// LRM 12.1: a module ends with endmodule
// expect: E4100 4:8 no endmodule
`timescale 1ns/1ps
module m;
  wire w;

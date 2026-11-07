// LRM 19.3.2: `undef removes a macro; its use afterwards is an error
// expect: E4007 7:9 macro was undefined
`timescale 1ns/1ps
`define W 8
`undef W
module m;
  wire [`W:0] w;
endmodule

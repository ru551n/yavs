// LRM 19.3: a macro defined after its use is not defined at the use
// expect: E4007 5:9 defined only later
`timescale 1ns/1ps
module m;
  wire [`LATE:0] w;
endmodule
`define LATE 3

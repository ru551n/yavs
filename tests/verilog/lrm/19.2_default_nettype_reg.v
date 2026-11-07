// LRM 19.2: reg is not a net type
// expect: E4009 4:1 reg is not a net type
`timescale 1ns/1ps
`default_nettype reg
module m;
endmodule

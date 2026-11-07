// LRM 19.1: `celldefine and `endcelldefine bracket modules to be treated as cells
// units: m
// expect: none
`timescale 1ns/1ps
`celldefine
module m;
endmodule
`endcelldefine

// LRM 19.3.2: `undef of a name that is not defined is not an error
// units: m
// expect: none
`timescale 1ns/1ps
`undef NEVER
module m;
endmodule

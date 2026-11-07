// LRM 19.10: a `pragma that the tool does not know is ignored
// units: m
// expect: none
`timescale 1ns/1ps
`pragma unknown_pragma_name value
module m;
endmodule

// LRM 28.1: an envelope is introduced by the protect pragma, so the //pragma protect form inside a comment is a comment; other tools write it that way, and yavs lexes the body (compatibility reading V9, not a gap)
// units: m
// expect: none
`timescale 1ns/1ps
module m (input a, output b);
//pragma protect begin_protected
endmodule

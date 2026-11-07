// LRM 28: the //pragma protect form inside a comment is a comment: not an envelope (known gap G3: other tools write it)
// units: m
// gap: G3 the comment form of the envelope is a plain comment here
// expect: none
`timescale 1ns/1ps
module m (input a, output b);
//pragma protect begin_protected
endmodule

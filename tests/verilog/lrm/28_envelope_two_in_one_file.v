// LRM 28: one W4007 is given per file however many envelopes it has
// units: m,n
// expect: W4007 6:1 encrypted module body
`timescale 1ns/1ps
module m (input a, output b);
`pragma protect begin_protected
AAAA
`pragma protect end_protected
endmodule
module n (input a);
`pragma protect begin_protected
BBBB
`pragma protect end_protected
endmodule

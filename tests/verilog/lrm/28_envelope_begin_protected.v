// LRM 28: an envelope opened by `pragma protect begin_protected hides its body: the module is recorded, one W4007 per file
// units: m
// expect: W4007 6:1 encrypted module body
`timescale 1ns/1ps
module m (input a, output b);
`pragma protect begin_protected
`pragma protect key_block
AAAA bbbb " unterminated /* not read
`pragma protect end_protected
endmodule

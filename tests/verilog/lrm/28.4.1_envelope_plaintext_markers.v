// LRM 28.4.1: `pragma protect begin and end mark plaintext to be encrypted: the text is compiled, with a warning per marker
// units: m
// expect: W4009 6:1 plaintext marker
// expect: W4009 9:1 plaintext marker
`timescale 1ns/1ps
`pragma protect begin
module m;
endmodule
`pragma protect end

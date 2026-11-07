// LRM 28.2: yavs records the module of an envelope that stands outside any module as no module, with one W4007 (tool behaviour, D8; 28.2.2 wants it decrypted)
// units: m
// expect: W4007 5:1 envelope outside a module
`timescale 1ns/1ps
`pragma protect begin_protected
AAAA
`pragma protect end_protected
module m;
endmodule

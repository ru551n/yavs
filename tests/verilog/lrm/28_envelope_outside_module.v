// LRM 28: an envelope that stands outside any module records no module
// units: m
// expect: W4007 5:1 envelope outside a module
`timescale 1ns/1ps
`pragma protect begin_protected
AAAA
`pragma protect end_protected
module m;
endmodule

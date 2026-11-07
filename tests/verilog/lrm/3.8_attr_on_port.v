// LRM 3.8: an attribute may precede an ANSI port declaration
// units: m
// expect: none
`timescale 1ns/1ps
module m ((* mark *) input a);
endmodule

// LRM 28: the older `protect and `endprotect pair hides a body in the same way
// units: m
// expect: W4007 6:1 encrypted module body
`timescale 1ns/1ps
module m (input a, output b);
`protect
xxxx " never closed
`endprotect
endmodule

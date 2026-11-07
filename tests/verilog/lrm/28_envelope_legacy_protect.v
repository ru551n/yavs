// LRM 28: not in 1364-2005, a vendor form: the older `protect and `endprotect pair hides a body like a begin_protected envelope
// units: m
// expect: W4007 6:1 encrypted module body
`timescale 1ns/1ps
module m (input a, output b);
`protect
xxxx " never closed
`endprotect
endmodule

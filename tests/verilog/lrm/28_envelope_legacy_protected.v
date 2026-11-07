// LRM 28: not in 1364-2005, a vendor form: `protected and `endprotected also open an envelope
// units: m
// expect: W4007 6:1 encrypted module body
`timescale 1ns/1ps
module m (input a, output b);
`protected
xxxx
`endprotected
endmodule

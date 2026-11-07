// LRM 28: not in 1364-2005; an envelope in the module header hides the header, and the plaintext after it is part of the module
// units: m
// expect: W4007 5:10 encrypted module body
`timescale 1ns/1ps
module m `protect (
xxxx
`endprotect
initial $display("x");
endmodule

// LRM 12.3.9.1: an inout port is a net: it cannot be declared reg
// expect: E4202 4:21 inout port declared reg
`timescale 1ns/1ps
module m (inout reg a);
endmodule

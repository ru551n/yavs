// LRM 12.3: an input port is a net: it cannot be declared reg
// expect: E4202 4:21 input port declared reg
`timescale 1ns/1ps
module m (input reg a);
endmodule

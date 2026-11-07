// LRM 19.3.1: `define is followed by a macro name
// expect: E4008 4:1 no macro name
`timescale 1ns/1ps
`define
module m;
endmodule

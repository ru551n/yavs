// LRM 12.1: parameters may be declared in the header of a module
// units: m
// expect: none
`timescale 1ns/1ps
module m #(parameter A = 1, parameter [3:0] B = 4'd2, C = 3) ();
endmodule

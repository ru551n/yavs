// LRM 19.4: a conditional started in an included file ends in the same file
// yavs-test: args=-I inc
// units: m
// expect: none
`timescale 1ns/1ps
module m;
`include "net.vh"
endmodule

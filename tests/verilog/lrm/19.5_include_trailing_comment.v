// LRM 19.5: a comment may follow `include on its line
// yavs-test: args=-I inc
// units: m
// expect: none
`timescale 1ns/1ps
module m;
`include "net.vh" // the net
endmodule

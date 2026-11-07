// LRM 19.5: `include may stand anywhere, here between module items
// yavs-test: args=-I inc
// units: m
// expect: none
`timescale 1ns/1ps
module m;
`include "net.vh"
endmodule

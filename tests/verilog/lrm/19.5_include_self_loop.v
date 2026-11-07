// LRM 19.5: files that include each other never finish
// yavs-test: args=-I inc
// expect: E4011 inc/loop.vh:1:1 include nested too deeply
`timescale 1ns/1ps
`include "loop.vh"
module m;
endmodule

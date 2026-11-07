// LRM 19.5: the file name of an `include is a quoted string
// yavs-test: args=-I inc
// expect: E4009 5:1 file name not quoted
`timescale 1ns/1ps
`include width.vh
module m;
endmodule

// LRM 19.7: `line sets the line number and file name of the next line
// units: m
// expect: none
`timescale 1ns/1ps
`line 100 "other.v" 0
module m;
endmodule

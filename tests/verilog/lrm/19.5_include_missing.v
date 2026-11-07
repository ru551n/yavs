// LRM 19.5: the file of an `include is found or the include is an error
// expect: E4005 4:1 file not found
`timescale 1ns/1ps
`include "no_such_file.vh"
module m;
endmodule

// LRM 19.3: default values of macro arguments are not in 1364-2005
// expect: E4003 4:13 default argument is SystemVerilog
`timescale 1ns/1ps
`define D(a = 1) a
module m;
endmodule

// LRM 12.3.4: only an output variable is given a value in the port list; a value on an input is SystemVerilog and accepted (V16)
// units: m
// expect: W4017 5:19 default value of a port
`timescale 1ns/1ps
module m (input a = 1'b0, input b = 1'b1, output reg c = 0);
endmodule

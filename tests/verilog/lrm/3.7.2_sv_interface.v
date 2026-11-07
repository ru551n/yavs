// LRM 3.7.2: interface is SystemVerilog
// expect: E4003 4:1 interface is SystemVerilog
`timescale 1ns/1ps
interface i;
endinterface
module m;
endmodule

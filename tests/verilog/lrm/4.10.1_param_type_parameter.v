// LRM 4.10.1: type parameters are SystemVerilog
// expect: E4003 5:13 type parameter is SystemVerilog
`timescale 1ns/1ps
module m;
  parameter type t = logic;
endmodule

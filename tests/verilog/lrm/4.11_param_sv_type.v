// LRM 4.11: a parameter type other than the 1364 ones (here int) is SystemVerilog
// expect: E4003 5:13 int is SystemVerilog
`timescale 1ns/1ps
module m;
  parameter int a = 1;
endmodule

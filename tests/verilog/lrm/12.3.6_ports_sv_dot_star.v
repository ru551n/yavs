// LRM 12.3.6: the .* port connection is SystemVerilog
// expect: E4003 8:8 .* is SystemVerilog
`timescale 1ns/1ps
module c (input a);
endmodule
module m;
  wire a;
  c u (.*);
endmodule

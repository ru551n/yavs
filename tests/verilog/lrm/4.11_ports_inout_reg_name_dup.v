// LRM 4.11: a name is declared once in a module scope
// expect: E4203 6:7 a declared twice
`timescale 1ns/1ps
module m;
  wire a;
  reg a;
endmodule

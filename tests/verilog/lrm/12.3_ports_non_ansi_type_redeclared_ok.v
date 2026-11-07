// LRM 12.3: a non-ANSI port may be declared again as reg or wire to give it a type
// units: m
// expect: none
`timescale 1ns/1ps
module m (a);
  output a;
  reg a;
endmodule

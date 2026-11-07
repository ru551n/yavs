// LRM 12.3.4: the body may repeat the name of an ANSI port to give it a type: accepted, as the other tools do (V6)
// units: m
// expect: none
`timescale 1ns/1ps
module m (output o);
  reg o;
endmodule

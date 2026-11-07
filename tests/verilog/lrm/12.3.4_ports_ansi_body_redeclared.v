// LRM 12.3.4: a port declared in the header is not declared again in the body; accepted with a warning (V6)
// units: m
// expect: W4209 6:7 output declared again
`timescale 1ns/1ps
module m (output o);
  reg o;
endmodule

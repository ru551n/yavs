// LRM 14.2: specparam may also be declared in the module body
// units: m
// expect: none
`timescale 1ns/1ps
module m;
  specparam tpd = 2;
endmodule

// LRM 4.6.3: trireg is a net type
// units: m
// expect: none
`timescale 1ns/1ps
module m;
  trireg n;
  trireg [7:0] v;
endmodule

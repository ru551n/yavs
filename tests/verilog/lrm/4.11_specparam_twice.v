// LRM 4.11: a name is declared once in a specify block
// expect: E4203 7:15 specparam again
`timescale 1ns/1ps
module m;
  specify
    specparam a = 1;
    specparam a = 2;
  endspecify
endmodule

// LRM 3.8: an attribute instance may follow an operator or the ? of a conditional
// units: m
// expect: none
`timescale 1ns/1ps
module m;
  integer a, b, c, d;
  initial begin
    a = b + (* mode = "cla" *) c;
    a = b ? (* no_glitch *) c : d;
  end
endmodule

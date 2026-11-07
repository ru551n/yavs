// LRM 4.6.5: a uwire net has one driver (known gap G11: checked when a design is elaborated)
// units: m
// gap: G11 rules that need the elaborated design are not checked
// expect: none
`timescale 1ns/1ps
module m;
  uwire w;
  assign w = 1'b0;
  assign w = 1'b1;
endmodule

// LRM 4.6.5: it is an error for a uwire to have more than one driver (elaboration of the module)
// expect: none
// elab: E4502
`timescale 1ns/1ps
module m;
  uwire w;
  assign w = 1'b0;
  assign w = 1'b1;
endmodule

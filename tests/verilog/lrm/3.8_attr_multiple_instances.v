// LRM 3.8: several attribute instances may precede one item, and a name may repeat
// units: m
// expect: none
`timescale 1ns/1ps
module m;
  (* a *) (* b = 1 *) wire w;
  (* c = 1, c = 2 *) wire v;
endmodule

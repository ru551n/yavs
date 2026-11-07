// LRM 12.3.5: instance ports connect by position, with an empty position allowed
// units: c,m
// expect: none
`timescale 1ns/1ps
module c (input a, output b);
endmodule
module m;
  wire x, y;
  c u (x, y);
  c v (x, );
endmodule

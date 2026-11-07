// LRM 12.3: instance ports connect by name, with empty connections allowed
// units: c,m
// expect: none
`timescale 1ns/1ps
module c (input a, output b);
endmodule
module m;
  wire x, y;
  c u (.a(x), .b());
  c v (.a(), .b(y));
endmodule

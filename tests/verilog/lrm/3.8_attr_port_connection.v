// LRM 3.8: an attribute instance may precede a port connection, ordered or by name
// units: c,m
// expect: none
`timescale 1ns/1ps
module c (input i, input j);
endmodule
module m;
  wire w;
  c u (.i((* a *) w), (* b *) .j(w));
endmodule

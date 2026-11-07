// LRM 7.1.6: gate terminals are positional: a named connection is an error
// expect: E4100 6:3 named connection on a gate
`timescale 1ns/1ps
module m;
  wire o, a, b;
  and g (.o(o), .a(a));
endmodule

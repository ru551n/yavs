// LRM 12.3.2: a port may be a named port expression such as .p(a)
// units: m
// expect: none
`timescale 1ns/1ps
module m (.p(a), .q({b, c}));
  input a;
  input b;
  output c;
endmodule

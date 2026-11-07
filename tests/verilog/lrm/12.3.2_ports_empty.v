// LRM 12.3.2: a port of the list may be empty: it has no name and nothing is connected to it
// units: m,n
// expect: none
`timescale 1ns/1ps
module m (a, , b);
  input a, b;
endmodule
module n (,);
endmodule

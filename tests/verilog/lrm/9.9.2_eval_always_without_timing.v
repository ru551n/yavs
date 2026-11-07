// LRM 9.9.2: an always block without a timing control never lets time advance
// expect: none
// elab: E4503
`timescale 1ns/1ps
module m;
  reg a;
  always a = ~a;
endmodule

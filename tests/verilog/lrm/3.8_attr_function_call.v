// LRM 3.8: an attribute instance may stand between a function name and its arguments
// units: m
// expect: none
`timescale 1ns/1ps
module m;
  integer a, b, c;
  function integer add (input integer x, input integer y); add = x + y; endfunction
  initial a = add (* mode = "cla" *) (b, c);
endmodule

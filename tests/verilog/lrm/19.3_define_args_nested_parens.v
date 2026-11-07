// LRM 19.3: a comma inside parentheses does not separate actual arguments
// units: m
// expect: none
`timescale 1ns/1ps
`define ID(x) x
module m;
  parameter p = `ID(f(1, 2));
  function integer f(input integer a, b); f = a; endfunction
endmodule

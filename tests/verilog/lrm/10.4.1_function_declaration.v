// LRM 10.4.1: a function has a return type or range and at least one input
// units: m
// expect: none
`timescale 1ns/1ps
module m;
  function [3:0] f;
    input [3:0] a;
    f = a;
  endfunction
  function integer g (input integer x);
    g = x;
  endfunction
  function automatic real h (input real x);
    h = x;
  endfunction
endmodule

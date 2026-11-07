// LRM 8.5: a sequential UDP may initialise its output with initial
// units: p
// expect: none
`timescale 1ns/1ps
primitive p (output reg q, input d, c);
  initial q = 1'b0;
  table
    1 (01) : ? : 1;
  endtable
endprimitive

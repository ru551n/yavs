// LRM 8.2: a combinational UDP has one output, then inputs, and a table of input values : output
// units: p
// expect: none
`timescale 1ns/1ps
primitive p (output o, input a, b);
  table
    0 0 : 0;
    0 1 : 1;
    1 0 : 1;
    1 1 : 0;
    x ? : x;
    ? x : x;
  endtable
endprimitive

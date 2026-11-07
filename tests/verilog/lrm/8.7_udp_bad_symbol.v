// LRM 8.7: a table symbol is one of 0 1 x X ? b B - r R f F p P n N *
// expect: E4100 5:8 q is not a table symbol
`timescale 1ns/1ps
primitive p (output o, input a);
  table
    q : 1;
  endtable
endprimitive

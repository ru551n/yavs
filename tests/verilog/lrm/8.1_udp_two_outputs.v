// LRM 8.1: a UDP has exactly one output
// expect: E4100 4:24 second output
`timescale 1ns/1ps
primitive p (output o, output q, input a);
  table
    0 : 1;
  endtable
endprimitive

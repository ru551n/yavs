// LRM 8.1.1: a UDP has no inout port
// expect: E4100 4:24 inout port
`timescale 1ns/1ps
primitive p (output o, inout a);
  table
    0 : 1;
  endtable
endprimitive

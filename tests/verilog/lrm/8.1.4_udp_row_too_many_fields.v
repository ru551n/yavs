// LRM 8.1.4: a combinational row has one value per input
// expect: E4100 5:8 two input values for one input
`timescale 1ns/1ps
primitive p (output o, input a);
  table
    0 0 : 1;
  endtable
endprimitive

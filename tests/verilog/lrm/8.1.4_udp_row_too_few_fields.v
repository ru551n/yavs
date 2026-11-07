// LRM 8.1.4: a combinational row has one value per input
// expect: E4100 5:8 one input value for two inputs
`timescale 1ns/1ps
primitive p (output o, input a, b);
  table
    0 : 1;
  endtable
endprimitive

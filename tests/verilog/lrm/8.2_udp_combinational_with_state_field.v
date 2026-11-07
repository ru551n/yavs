// LRM 8.2: a combinational row has no current-state field
// expect: E4100 5:8 state field in combinational UDP
`timescale 1ns/1ps
primitive p (output o, input a);
  table
    0 : 0 : 1;
  endtable
endprimitive

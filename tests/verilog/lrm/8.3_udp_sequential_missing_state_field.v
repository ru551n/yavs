// LRM 8.3: a sequential row has a current-state field
// expect: E4100 5:8 no state field
`timescale 1ns/1ps
primitive p (output reg q, input d);
  table
    0 : 1;
  endtable
endprimitive

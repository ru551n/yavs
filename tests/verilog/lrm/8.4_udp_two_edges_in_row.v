// LRM 8.4: a row has at most one edge
// expect: E4100 5:8 two edges in one row
`timescale 1ns/1ps
primitive p (output reg q, input a, b);
  table
    (01) (01) : ? : 1;
  endtable
endprimitive

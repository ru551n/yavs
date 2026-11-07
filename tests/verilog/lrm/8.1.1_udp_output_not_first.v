// LRM 8.1.1: the output of a UDP is its first port
// expect: E4100 4:14 output is not first
`timescale 1ns/1ps
primitive p (input a, output o);
  table
    0 : 1;
  endtable
endprimitive

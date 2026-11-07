// LRM 8.1.1: a UDP header may list names with the directions declared in the body
// units: p
// expect: none
`timescale 1ns/1ps
primitive p (o, a, b);
  output o;
  input a, b;
  table
    0 0 : 0;
    1 1 : 1;
  endtable
endprimitive

// LRM 8.5: the initial value of a UDP is 0, 1 or x
// expect: E4100 5:15 initial value 2
`timescale 1ns/1ps
primitive p (output reg q, input d, c);
  initial q = 2;
  table
    1 (01) : ? : 1;
  endtable
endprimitive

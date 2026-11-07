// LRM 8.3: initial is optional for a sequential UDP
// units: p
// expect: none
`timescale 1ns/1ps
primitive p (output reg q, input d);
  table
    0 : ? : 0;
    1 : ? : 1;
  endtable
endprimitive

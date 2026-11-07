// LRM 8.3: a sequential UDP has an output reg and rows of inputs : state : next state
// units: latch
// expect: none
`timescale 1ns/1ps
primitive latch (output reg q, input d, en);
  table
    1 1 : ? : 1;
    0 1 : ? : 0;
    ? 0 : ? : -;
  endtable
endprimitive

// LRM 8.4: an edge is two values in parentheses, or one of r, f, p, n, *
// units: dff
// expect: none
`timescale 1ns/1ps
primitive dff (output reg q, input d, clk);
  table
    0 (01) : ? : 0;
    1 (01) : ? : 1;
    ? (1?) : ? : -;
    ? (?0) : ? : -;
    ? r : ? : -;
    ? f : ? : -;
    ? p : ? : -;
    ? n : ? : -;
    ? * : ? : -;
  endtable
endprimitive

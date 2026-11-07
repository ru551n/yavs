// LRM 8.1.6: - is not permitted in an input field
// expect: E4100 4:8 dash input
primitive p (output reg q, input a);
  table
    - : 0 : 1;
  endtable
endprimitive

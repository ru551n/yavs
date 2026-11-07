// LRM 8.1.6: an edge is only permitted in the input field
// expect: E4100 4:8 edge in output
primitive p (output reg q, input a);
  table
    0 : 0 : (01);
  endtable
endprimitive

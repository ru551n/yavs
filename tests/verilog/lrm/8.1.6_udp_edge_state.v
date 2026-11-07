// LRM 8.1.6: the current state is a level, not an edge
// expect: E4100 4:8 edge in state
primitive p (output reg q, input a);
  table
    0 : (01) : 1;
  endtable
endprimitive

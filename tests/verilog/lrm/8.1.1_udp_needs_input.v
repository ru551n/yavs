// LRM 8.1.1: a UDP has at least one input
// expect: E4100 3:11 no input
primitive p (output o);
  table
    : 1;
  endtable
endprimitive

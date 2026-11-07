// LRM 8.1.6: a combinational UDP has levels only
// expect: E4100 4:8 edge in combinational
primitive p (output o, input a);
  table
    (01) : 1;
  endtable
endprimitive

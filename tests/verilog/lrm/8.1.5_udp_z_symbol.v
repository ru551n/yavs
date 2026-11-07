// LRM 8.1.5: z is not a table symbol
// expect: E4100 4:8 z in table
primitive p (output o, input a);
  table
    z : 1;
  endtable
endprimitive

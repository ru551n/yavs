// LRM 8.1.6: b is not permitted in the output field
// expect: E4100 4:8 b output
primitive p (output o, input a);
  table
    0 : b;
  endtable
endprimitive

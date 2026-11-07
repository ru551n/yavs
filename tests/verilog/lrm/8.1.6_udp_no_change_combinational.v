// LRM 8.1.6: - is only permitted in the output field of a sequential UDP
// expect: E4100 4:8 dash in combinational
primitive p (output o, input a);
  table
    0 : -;
  endtable
endprimitive

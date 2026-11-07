// LRM 8.1.6: ? is not permitted in the output field of a UDP table
// expect: E4100 4:8 question mark output
primitive p (output o, input a);
  table
    0 : ?;
  endtable
endprimitive

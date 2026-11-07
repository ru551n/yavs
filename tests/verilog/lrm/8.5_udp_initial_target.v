// LRM 8.5: the initial statement of a UDP assigns its output
// expect: E4100 4:11 not the output
primitive p (output reg q, input a, b);
  initial b = 0;
  table
    0 0 : 0 : 1;
  endtable
endprimitive

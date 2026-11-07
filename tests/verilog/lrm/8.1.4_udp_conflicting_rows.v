// LRM 8.1.4: the same input combination is not given two different outputs (known gap G3: rows are not compared)
// gap: G3 rows that give the same inputs two outputs are accepted
// expect: none
primitive p (output o, input a);
  table
    0 : 1;
    0 : 0;
  endtable
endprimitive

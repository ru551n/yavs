// LRM 3.8: an attribute instance may precede the port declarations in a UDP header
// expect: none
primitive p ((* a *) output o, (* b *) input i);
  table
    0 : 1;
  endtable
endprimitive

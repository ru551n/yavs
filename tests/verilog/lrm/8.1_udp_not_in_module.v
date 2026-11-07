// LRM 8.1: a UDP is declared outside a module
// expect: E4100 4:3 primitive in a module
module m;
  primitive p (output o, input a);
    table
      0 : 1;
    endtable
  endprimitive
endmodule

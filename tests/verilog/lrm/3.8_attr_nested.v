// LRM 3.8: attribute instances do not nest: a value is not an attribute instance
// expect: E4100 4:18 attribute in a value
module m;
  (* a = (* b *) 1 *) wire w;
endmodule

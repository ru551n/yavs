// LRM 9.3: the target of an assign statement is not a bit-select, part-select or array element
// expect: E4100 5:11 bit select target
module m;
  reg [3:0] r;
  initial assign r[0] = 1'b1;
endmodule

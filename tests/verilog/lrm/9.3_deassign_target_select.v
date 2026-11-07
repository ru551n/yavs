// LRM 9.3: the target of a deassign statement is a whole variable
// expect: E4100 5:11 part select target
module m;
  reg [3:0] r;
  initial deassign r[1:0];
endmodule

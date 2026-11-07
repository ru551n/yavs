// LRM 4.2.1: a trireg with a charge strength is declared without an assignment
// expect: E4100 4:10 charge strength with assignment
module m;
  trireg (small) t = 1'b1;
endmodule

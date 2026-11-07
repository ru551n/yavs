// LRM 5.1.14: a replication is not the target of an assignment
// expect: E4100 5:10 replication on the left
module m;
  wire a;
  assign {2{a}} = 2'b00;
endmodule

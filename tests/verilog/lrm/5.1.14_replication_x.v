// LRM 5.1.14: a replication constant is not x or z
// expect: E4100 5:15 replication of z
module m;
  wire [3:0] w;
  assign w = {1'bz{1'b0}};
endmodule

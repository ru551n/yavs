// LRM 9.8.1: a sequential block has declarations only if it is named
// expect: E4100 5:5 declaration in unnamed block
module m;
  initial begin
    reg r;
    r = 1;
  end
endmodule

// LRM 6.2.1: a variable declaration assignment is only allowed at module level
// expect: E4100 5:13 initial value in a block
module m;
  initial begin : b
    integer i = 0;
  end
endmodule

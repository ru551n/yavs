// LRM 17: a system task enable is a statement and does not stand at module level (other than the elaboration tasks of V4)
// expect: E4100 5:3 system task at module level
`timescale 1ns/1ps
module m;
  $display("x");
endmodule

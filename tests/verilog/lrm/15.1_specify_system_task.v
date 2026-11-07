// LRM 15.1: no system task can appear in a specify block
// expect: E4100 5:5 display in specify
module m;
  specify
    $display("x");
  endspecify
endmodule

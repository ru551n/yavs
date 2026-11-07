// LRM 12.4: a generate block has no parameter declaration
// expect: E4100 5:5 parameter in generate
module m;
  if (1) begin : g
    parameter p = 1;
  end
endmodule

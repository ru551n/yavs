// LRM 12.4: a generate block has no specify block
// expect: E4100 5:5 specify in generate
module m;
  if (1) begin : g
    specify
    endspecify
  end
endmodule

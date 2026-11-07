// LRM 12.4: a generate block has no specparam declaration
// expect: E4100 5:5 specparam in generate
module m;
  if (1) begin : g
    specparam sp = 1;
  end
endmodule

// LRM 12.4: a generate block has no port declaration
// expect: E4100 5:5 input in generate
module m;
  if (1) begin : g
    input x;
  end
endmodule

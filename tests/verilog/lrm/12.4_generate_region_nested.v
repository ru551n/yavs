// LRM 12.4: generate regions do not nest
// expect: E4100 5:5 generate in generate
module m;
  generate
    generate
    endgenerate
  endgenerate
endmodule

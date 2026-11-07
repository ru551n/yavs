// LRM 14.2.4: ifnone goes with a simple path, not an edge-sensitive one
// expect: E4100 5:5 ifnone with an edge
module m (input clk, d, output q);
  specify
    ifnone (posedge clk => (q : d)) = 1;
  endspecify
endmodule

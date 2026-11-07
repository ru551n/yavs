// LRM 12.4.2: two conditional generate constructs do not use the same block name
// expect: E4203 8:23 block name again
`timescale 1ns/1ps
module m;
  parameter p = 0;
  if (p == 1) begin : a
  end
  if (p == 2) begin : a
  end
endmodule

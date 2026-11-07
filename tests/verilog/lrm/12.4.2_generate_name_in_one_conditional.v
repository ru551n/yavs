// LRM 12.4.2: the blocks of one conditional generate construct may share a name
// units: m
// expect: none
`timescale 1ns/1ps
module m;
  parameter p = 0;
  if (p == 1) begin : a
  end else if (p == 2) begin : a
  end else begin : a
  end
endmodule

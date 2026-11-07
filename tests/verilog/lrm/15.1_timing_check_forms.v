// LRM 15.1: the timing check system tasks take a data event, a reference event and limits
// units: m
// expect: none
`timescale 1ns/1ps
module m (clk, d, q);
  input clk, d, q;
  specify
    $setuphold(posedge clk, d, 1, 2);
    $recovery(posedge clk, d, 3);
    $removal(posedge clk, d, 3);
    $recrem(posedge clk, d, 1, 2);
    $skew(posedge clk, d, 2);
    $width(posedge clk, 5);
    $period(posedge clk, 10);
    $nochange(posedge clk, d, 0, 0);
    $setup(d, posedge clk &&& q, 1);
  endspecify
endmodule

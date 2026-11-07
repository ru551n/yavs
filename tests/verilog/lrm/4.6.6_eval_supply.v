// LRM 4.6.6: a supply0 net is always 0 and a supply1 net always 1, whatever drives them
// expect: none
// output: 0 1 00 11
`timescale 1ns/1ps
module m;
  reg a;
  supply0 gnd;
  supply1 vdd;
  supply0 [1:0] gnds;
  supply1 [1:0] vdds;
  assign gnd = 1'b1;
  assign vdds = {a, 1'b0};
  initial begin
    a = 0;
    #1 $display("%b %b %b %b", gnd, vdd, gnds, vdds);
  end
endmodule

// LRM 4.6.1: drivers that give z yield to the one that drives; a bus nobody drives is z, and two enabled drivers of different values give x
// expect: none
// output: z 1 0 x z
`timescale 1ns/1ps
module m;
  reg ea, eb, da, db;
  wire bus;
  assign bus = ea ? da : 1'bz;
  assign bus = eb ? db : 1'bz;
  initial begin
    ea = 0; eb = 0; da = 1; db = 0;
    #1 $write("%b ", bus);
    ea = 1;
    #1 $write("%b ", bus);
    ea = 0; eb = 1;
    #1 $write("%b ", bus);
    ea = 1;
    #1 $write("%b ", bus);
    ea = 0; eb = 0;
    #1 $display("%b", bus);
  end
endmodule

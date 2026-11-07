// LRM 4.6.4: a tri0 net is 0 and a tri1 net is 1 when nothing drives it; a driver that gives z leaves the pull, any other value wins over it
// expect: none
// output: 0 1 | 0 1 | 1 1 | x x | 0 1
`timescale 1ns/1ps
module m;
  reg a;
  tri0 free0;
  tri1 free1;
  tri0 [1:0] n0;
  tri1 n1;
  tri1 [1:0] bus;
  assign n0[0] = a;
  assign n1 = a;
  assign bus[0] = a;
  initial begin
    $write("%b %b | ", free0, free1);
    a = 1'bz;
    #1 $write("%b %b | ", n0[0], n1);
    a = 1'b1;
    #1 $write("%b %b | ", n0[0], n1);
    a = 1'bx;
    #1 $write("%b %b | ", n0[0], n1);
    a = 1'bz;
    #1 $display("%b %b", n0[1], bus[1]);
  end
endmodule

// LRM 18.1: value change dump tasks
// units: m
// expect: none
`timescale 1ns/1ps
module m;
  reg a;
  initial begin
    $dumpfile("x.vcd");
    $dumpvars(0, a);
    $dumpon; $dumpoff; $dumpall; $dumpflush;
  end
endmodule

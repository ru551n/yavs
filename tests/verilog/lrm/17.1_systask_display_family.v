// LRM 17.1: the display, write, strobe and monitor tasks, with base suffixes
// units: m
// expect: none
`timescale 1ns/1ps
module m;
  reg [7:0] a;
  initial begin
    $display("a=%h", a);
    $displayb(a); $displayo(a); $displayh(a);
    $write("x"); $writeb(a);
    $strobe("s"); $strobeh(a);
    $monitor("m", a); $monitoron; $monitoroff;
  end
endmodule

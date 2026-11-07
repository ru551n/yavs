// LRM 9.7: delay, event and wait controls
// units: m
// expect: none
`timescale 1ns/1ps
module m;
  reg a, b, c;
  event e;
  initial begin
    #1;
    #(1.5);
    @(posedge a);
    @(a or b);
    @(a, b);
    @(negedge a or posedge b);
    @e;
    wait (a) b = 1;
    -> e;
  end
endmodule

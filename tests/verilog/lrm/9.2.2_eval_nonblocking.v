// LRM 9.2.2: a non-blocking assignment takes its value when it runs and updates after the active events, so a swap works
// expect: none
// output: 1
// output: 2 1
`timescale 1ns/1ps
module m;
  reg [3:0] a, b;
  initial begin
    a = 1;
    b = 2;
    a <= b;
    b <= a;
    $display("%0d", a);
    #1 $display("%0d %0d", a, b);
  end
endmodule

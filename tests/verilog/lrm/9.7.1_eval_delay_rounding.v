// LRM 9.7.1: a delay is in the module's time unit and rounded to its precision; $time is rounded to the unit; %t shows the smallest precision
// expect: none
// output: 2 20 1600
`timescale 1ns/100ps
module m;
  initial begin
    #1.55 $display("%0d %0t %0d", $time, $time, $realtime * 1000);
  end
endmodule

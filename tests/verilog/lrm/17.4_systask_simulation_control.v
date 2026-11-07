// LRM 17.4: $finish and $stop take an optional argument
// units: m
// expect: none
`timescale 1ns/1ps
module m;
  initial begin
    $finish;
    $finish(1);
    $stop;
    $stop(0);
  end
endmodule

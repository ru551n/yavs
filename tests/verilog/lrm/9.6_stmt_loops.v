// LRM 9.6: forever, repeat, while and for loops
// units: m
// expect: none
`timescale 1ns/1ps
module m;
  integer i;
  reg a;
  initial begin
    for (i = 0; i < 4; i = i + 1) a = ~a;
    while (a) a = 0;
    repeat (3) a = ~a;
    forever #1 a = ~a;
  end
endmodule

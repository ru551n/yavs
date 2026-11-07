// LRM 5.4.1: the right side of an assignment is evaluated at the width of the wider side, so a carry survives in a wider target
// expect: none
// output: 9 256
// output: 8 0
`timescale 1ns/1ps
module m;
  reg [7:0] a, b;
  reg [8:0] s9;
  reg [7:0] s8;
  initial begin
    a = 8'hff;
    b = 8'h01;
    s9 = a + b;
    s8 = a + b;
    $display("9 %0d", s9);
    $display("8 %0d", s8);
  end
endmodule

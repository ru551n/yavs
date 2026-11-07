// LRM 9.7.5: @* waits for every net and variable the statement reads
// expect: none
// output: 0 1 0
`timescale 1ns/1ps
module m;
  reg a, b, y;
  always @* y = a & b;
  initial begin
    a = 0;
    b = 1;
    #1 $write("%b ", y);
    a = 1;
    #1 $write("%b ", y);
    b = 0;
    #1 $display("%b", y);
  end
endmodule

// LRM 5.1.5: an x or z bit in an operand of an arithmetic operator makes the whole result x, and so does division by zero
// expect: none
// output: xxxx xxxx 0011
`timescale 1ns/1ps
module m;
  reg [3:0] a, b, c, d;
  initial begin
    a = 4'b10x1;
    b = a + 1;
    c = 4'd7 / 4'd0;
    d = 4'd7 / 4'd2;
    $display("%b %b %b", b, c, d);
  end
endmodule

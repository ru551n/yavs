// LRM 17.1.1: %b, %h, %o and %d show x and z per digit (lower case when the whole digit is, upper case when part of it is); %0 drops the padding
// expect: none
// output: 1010x0z1|aX|2XZ|  X
// output: 5|  5|0ab|ab|  -5
`timescale 1ns/1ps
module m;
  reg [7:0] v = 8'b1010x0z1;
  reg signed [7:0] n = -5;
  initial begin
    $display("%b|%h|%o|%d", v, v, v, v);
    $display("%0d|%d|%h|%0h|%d", 8'd5, 8'd5, 12'h0ab, 12'h0ab, n);
  end
endmodule

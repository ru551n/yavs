// LRM 5.5.2: an operand is sign-extended when the expression is signed and zero-extended when it is not
// expect: none
// output: -2 254 14
`timescale 1ns/1ps
module m;
  reg signed [3:0] a = -2;
  reg signed [7:0] s;
  reg [7:0] u, z;
  initial begin
    s = a;
    u = a;
    z = a + 4'd0;
    $display("%0d %0d %0d", s, u, z);
  end
endmodule

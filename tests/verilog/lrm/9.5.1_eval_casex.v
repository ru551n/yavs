// LRM 9.5.1: casex treats x and z bits on either side as don't care; case compares with ===
// expect: none
// output: 1 3
`timescale 1ns/1ps
module m;
  reg [1:0] r, q;
  initial begin
    casex (4'b10x1)
      4'b1001: r = 1;
      default: r = 0;
    endcase
    case (4'b10x1)
      4'b1001: q = 1;
      4'b10x1: q = 3;
      default: q = 0;
    endcase
    $display("%0d %0d", r, q);
  end
endmodule

`timescale 1ns/1ps
module counter #(parameter WIDTH = 4) (
  input clk,
  input rst,
  output reg [WIDTH-1:0] q
);
  always @(posedge clk or posedge rst)
    if (rst) q <= 0;
    else q <= q + 1;
endmodule

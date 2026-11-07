// LRM 4.6.1: drivers of different bits of a net do not meet, drivers that overlap resolve bit by bit
// expect: none
// output: 0110 01x0 z
`timescale 1ns/1ps
module m;
  reg [1:0] a, b;
  reg c, d;
  wire [3:0] w;
  wire [3:0] v;
  wire [1:0] u;
  assign w[1:0] = a;
  assign w[3:2] = b;
  assign v[1:0] = a;
  assign v[2:1] = {c, d};
  assign v[3] = 1'b0;
  assign u[1] = 1'bz;
  initial begin
    a = 2'b10; b = 2'b01; c = 1; d = 0;
    #1 $display("%b %b %b", w, v, u[1]);
  end
endmodule

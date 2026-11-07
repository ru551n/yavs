// LRM 4.6.1: two drivers of a wire or tri net resolve as the truth table of the clause: equal values stay, 0 and 1 give x, z yields to the other driver, x with anything but z is x
// expect: none
// output: 0xx0 1x1x xxxx zx10
// output: 0xx0 1x1x xxxx zx10
`timescale 1ns/1ps
module m;
  // Four groups of four pairs: the first driver is 0, 1, x and z; the second one, within a group, z, x, 1 and 0.
  reg [15:0] a, b;
  wire [15:0] w;
  tri [15:0] t;
  assign w = a;
  assign w = b;
  assign t = b;
  assign t = a;
  initial begin
    a = {4'b0000, 4'b1111, 4'bxxxx, 4'bzzzz};
    b = {4{4'bzx10}};
    #1;
    $display("%b %b %b %b", w[15:12], w[11:8], w[7:4], w[3:0]);
    $display("%b %b %b %b", t[15:12], t[11:8], t[7:4], t[3:0]);
  end
endmodule

// LRM 4.4.2: the stronger of two drivers wins, two weak drivers of different values give x, a highz side drives z
// expect: none
// output: 0 1 x 1 z 0 0 1
`timescale 1ns/1ps
module m;
  reg s0, s1, w0, w1;
  wire a, b, c, d, e, f;
  wire (weak1, weak0) g = w1;
  tri (weak1, strong0) h = w1;
  assign a = s0;
  assign (weak1, weak0) a = w1;
  assign b = s1;
  assign (weak1, weak0) b = w0;
  assign (weak1, weak0) c = w0;
  assign (weak1, weak0) c = w1;
  assign (weak1, weak0) d = w1;
  assign (highz1, strong0) e = w1;
  assign (highz1, strong0) f = w0;
  assign g = s0;
  initial begin
    s0 = 0; s1 = 1; w0 = 0; w1 = 1;
    #1 $display("%b %b %b %b %b %b %b %b", a, b, c, d, e, f, g, h);
  end
endmodule

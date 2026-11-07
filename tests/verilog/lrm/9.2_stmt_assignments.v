// LRM 9.2: blocking and non-blocking assignments, with intra-assignment delay
// units: m
// expect: none
`timescale 1ns/1ps
module m;
  reg a, b;
  initial begin
    a = 1;
    a = #1 b;
    a <= b;
    a <= #2 b;
    a = @(b) 1;
    a = repeat (2) @(b) 1;
  end
endmodule

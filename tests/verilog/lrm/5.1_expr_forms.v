// LRM 5.1: part selects, indexed part selects, concatenation, replication, conditional, function and system function calls
// units: m
// expect: none
`timescale 1ns/1ps
module m;
  reg [15:0] v, w;
  integer i, k;
  function integer f (input integer x); f = x; endfunction
  initial begin
    v = w[7:0];
    v = w[i+:4];
    v = w[15-:4];
    v = {w[3:0], 4'b0};
    v = {4{w[3:0]}};
    k = i ? f(1) : $clog2(i);
    k = $signed(v) + $unsigned(w);
    k = $time + $random;
  end
endmodule

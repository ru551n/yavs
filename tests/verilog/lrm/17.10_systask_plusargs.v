// LRM 17.10: $test$plusargs and $value$plusargs
// units: m
// expect: none
`timescale 1ns/1ps
module m;
  integer v;
  initial begin
    if ($test$plusargs("x")) v = 1;
    if ($value$plusargs("n=%d", v)) v = 2;
  end
endmodule

// LRM 9.4: else follows an if
// expect: E4100 7:5 else without if
`timescale 1ns/1ps
module m;
  reg a;
  initial begin
    else a = 1;
  end
endmodule

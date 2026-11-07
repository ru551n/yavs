// LRM 17.3: $printtimescale and $timeformat
// units: m
// expect: none
`timescale 1ns/1ps
module m;
  initial begin
    $printtimescale;
    $timeformat(-9, 2, " ns", 10);
  end
endmodule

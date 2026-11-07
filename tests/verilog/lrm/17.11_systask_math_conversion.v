// LRM 17.11: the integer and real math functions ($clog2, $sqrt, ...) and the conversion functions of 17.8 ($rtoi, $itor, $realtobits, $bitstoreal)
// units: m
// expect: none
`timescale 1ns/1ps
module m;
  real r;
  integer i;
  initial begin
    i = $clog2(10);
    r = $sqrt(2.0) + $pow(2.0, 3.0) + $ln(1.0) + $sin(0.0);
    i = $rtoi(r);
    r = $itor(i);
    i = $bitstoreal(1) + $realtobits(r);
  end
endmodule

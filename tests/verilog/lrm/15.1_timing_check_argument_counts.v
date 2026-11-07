// LRM 15.1: the optional arguments of the timing checks may be given or left empty
// units: m
// expect: none
`timescale 1ns/1ps
module m (input d, clk);
  reg n;
  wire dc, dd;
  specify
    $setup(d, posedge clk, 1);
    $setup(d, posedge clk, 1, n);
    $hold(posedge clk, d, 1, );
    $setuphold(posedge clk, d, 1, 2, n, , , dc, dd);
    $recrem(posedge clk, d, 1, 2, n);
    $skew(posedge clk, d, 5, n);
    $timeskew(posedge clk, d, 5, n, 1, 1);
    $fullskew(posedge clk, d, 5, 6, n, 0, 1);
    $width(posedge clk, 5, 0, n);
    $period(negedge clk, 10, n);
    $nochange(posedge clk, d, 0, 0, n);
  endspecify
endmodule
